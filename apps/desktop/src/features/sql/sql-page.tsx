import {
  type KeyboardEvent as ReactKeyboardEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import type { CatalogSnapshot, ConnectionId, QueryRun, SavedQuery } from '@aidb/contracts'
import type { SQLNamespace } from '@codemirror/lang-sql'
import { setDiagnostics } from '@codemirror/lint'
import type { EditorView } from '@codemirror/view'
import { useQueryClient } from '@tanstack/react-query'
import {
  LibraryBigIcon,
  ListTreeIcon,
  PlayIcon,
  PlusIcon,
  SaveIcon,
  SquareIcon,
  SquareTerminalIcon,
  XIcon,
} from 'lucide-react'
import { toast } from 'sonner'
import { PageContainer } from '@/components/shared/page'
import { ConnectionRequired, EmptyState } from '@/components/shared/status-states'
import { Button } from '@/components/ui/button'
import { Kbd } from '@/components/ui/kbd'
import { Spinner } from '@/components/ui/spinner'
import { useActiveConnection } from '@/features/connections/sessions'
import { useCatalog } from '@/features/data/queries'
import { analyzeSql, cancelOperation, runQuery, saveQuery } from '@/lib/commands'
import { NativeError } from '@/lib/native'
import { cn } from '@/lib/utils'
import { useEditorTabs } from './editor-store'
import { LibrarySheet } from './library-sheet'
import { HISTORY_KEY, SAVED_KEY } from './library-view'
import { pageTrail, shiftPosition } from './paging'
import { QueryResultView } from './query-result-view'
import { useTabRuns, type DoneRun, type DraftPosition } from './run-store'
import { PlanSheet } from './plan-sheet'
import { docPosition, problemRange, statementAt } from './run-target'
import { SaveQueryDialog } from './save-query-dialog'
import { SqlEditor } from './sql-editor'
import { MAX_TABS } from './tabs'

const isMac = navigator.userAgent.includes('Mac')
const MOD = isMac ? '⌘' : 'Ctrl'

export function SqlPage() {
  const active = useActiveConnection()
  if (!active) {
    return (
      <PageContainer>
        <ConnectionRequired
          icon={SquareTerminalIcon}
          title="Write and run SQL"
          description="Connect to a PostgreSQL database to run queries. The editor runs read statements; changes go through preview and approval."
        />
      </PageContainer>
    )
  }
  return <SqlWorkspace connectionId={active.id} />
}

/** The tab shown now, read when an async run finishes, so it is never a stale closure. */
function shownTab(): string {
  return useEditorTabs.getState().activeId
}

/** Tables of the open connection, for completion: `schema → table → []`. */
function namespace(catalog: CatalogSnapshot | undefined): SQLNamespace {
  const schemas: Record<string, Record<string, string[]>> = {}
  for (const object of catalog?.objects ?? []) {
    ;(schemas[object.schema] ??= {})[object.name] = []
  }
  return schemas
}

/** Puts the cursor at a draft position, clamped to the text as it is now. */
function revealPosition(view: EditorView, at: DraftPosition) {
  const { doc } = view.state
  const line = doc.line(Math.min(Math.max(at.line, 1), doc.lines))
  const pos = Math.min(line.from + at.column - 1, line.to)
  view.dispatch({ selection: { anchor: pos }, scrollIntoView: true })
  view.focus()
}

function SqlWorkspace({ connectionId }: { connectionId: ConnectionId }) {
  const { tabs, activeId, add, close, select, cycle, update, linkSaved } = useEditorTabs()
  const queryClient = useQueryClient()
  const [libraryOpen, setLibraryOpen] = useState(false)
  const [saveOpen, setSaveOpen] = useState(false)
  const runs = useTabRuns((state) => state.runs)
  const setRun = useTabRuns((state) => state.set)
  const forgetRun = useTabRuns((state) => state.forget)
  const viewRef = useRef<EditorView | null>(null)
  const tablistRef = useRef<HTMLDivElement>(null)
  const catalog = useCatalog(connectionId)
  const schema = useMemo(() => namespace(catalog.data), [catalog.data])
  const activeTab = tabs.find((tab) => tab.id === activeId) ?? tabs[0]
  const run = runs[activeId]
  // Results belong to the connection they ran on.
  const shownRun = run && run.connectionId === connectionId ? run : undefined
  const running = shownRun?.status === 'running'
  // While another page loads, the page being left stays on screen.
  const doneRun = shownRun?.status === 'done' ? shownRun : running ? shownRun.shown : undefined

  const runCurrent = useCallback(async () => {
    const view = viewRef.current
    const tabId = shownTab()
    if (!view || useTabRuns.getState().runs[tabId]?.status === 'running') return
    const doc = view.state.doc.toString()
    const selection = view.state.selection.main
    let text: string
    let base: number
    if (selection.empty) {
      let analysis
      try {
        analysis = await analyzeSql(doc)
      } catch (error) {
        toast.error('Could not read the SQL', { description: String(error) })
        return
      }
      if (analysis.problem) {
        finish(view, tabId, doc, 0, doc, {
          status: 'refused',
          message: analysis.problem.message,
          kind: null,
          position: analysis.problem.position,
        })
        return
      }
      const statement = statementAt(analysis, selection.head)
      if (!statement) {
        toast('Nothing to run', { description: 'Write a statement first.' })
        return
      }
      base = statement.span.start.offset
      text = doc.slice(base, statement.span.end.offset)
    } else {
      base = selection.from
      text = doc.slice(selection.from, selection.to)
    }

    const operationId = crypto.randomUUID()
    setRun(tabId, { status: 'running', connectionId, operationId })
    view.dispatch(setDiagnostics(view.state, []))
    try {
      const outcome = await runQuery(connectionId, { sql: text, operationId })
      finish(view, tabId, doc, base, text, outcome)
    } catch (error) {
      forgetRun(tabId)
      toast.error('Could not run the statement', {
        description: error instanceof Error ? error.message : String(error),
      })
    } finally {
      // The core records each run in the history.
      void queryClient.invalidateQueries({ queryKey: [HISTORY_KEY, connectionId] })
    }

    function finish(
      view: EditorView,
      tabId: string,
      doc: string,
      base: number,
      ran: string,
      outcome: QueryRun,
    ) {
      const problem = outcome.status === 'completed' ? null : outcome
      setRun(tabId, {
        status: 'done',
        connectionId,
        outcome,
        text: ran,
        start: docPosition(doc, base),
        problemAt: problem?.position ? docPosition(doc, base + problem.position.offset) : null,
        previous: [],
      })
      // Mark the problem only where it still applies: the same tab, the same text.
      if (problem?.position && shownTab() === tabId && view.state.doc.toString() === doc) {
        const { from, to } = problemRange(doc, base, problem.position)
        view.dispatch(
          setDiagnostics(view.state, [
            {
              from,
              to,
              severity: problem.status === 'failed' ? 'error' : 'warning',
              message: problem.message,
            },
          ]),
        )
      }
    }
  }, [connectionId, forgetRun, queryClient, setRun])

  /** Runs the same statement again for the next or previous page. */
  const loadPage = useCallback(
    async (direction: 'next' | 'previous') => {
      const tabId = shownTab()
      const current = useTabRuns.getState().runs[tabId]
      if (current?.status !== 'done' || current.outcome.status !== 'completed') return
      const trail = pageTrail(current.outcome, current.previous, direction)
      if (!trail) return
      const operationId = crypto.randomUUID()
      setRun(tabId, { status: 'running', connectionId, operationId, shown: current })
      let outcome: QueryRun
      try {
        outcome = await runQuery(connectionId, {
          sql: current.text,
          offset: trail.offset,
          operationId,
        })
      } catch (error) {
        setRun(tabId, current)
        toast.error('Could not load the page', {
          description: error instanceof Error ? error.message : String(error),
        })
        return
      }
      if (outcome.status === 'failed' && outcome.code === 'cancelled') {
        setRun(tabId, current)
        return
      }
      const position = outcome.status === 'completed' ? null : outcome.position
      const next: DoneRun = {
        ...current,
        outcome,
        problemAt: position ? shiftPosition(current.start, position) : null,
        previous: trail.previous,
      }
      setRun(tabId, next)
    },
    [connectionId, setRun],
  )

  // The query shown in the plan sheet: the selection, or the statement at the cursor.
  const [explaining, setExplaining] = useState<string | null>(null)
  const explainCurrent = useCallback(async () => {
    const view = viewRef.current
    if (!view) return
    const doc = view.state.doc.toString()
    const selection = view.state.selection.main
    if (!selection.empty) {
      setExplaining(doc.slice(selection.from, selection.to))
      return
    }
    try {
      const analysis = await analyzeSql(doc)
      if (analysis.problem) {
        toast.error('Could not read the SQL', { description: analysis.problem.message })
        return
      }
      const statement = statementAt(analysis, selection.head)
      if (!statement) {
        toast('Nothing to explain', { description: 'Write a query first.' })
        return
      }
      setExplaining(doc.slice(statement.span.start.offset, statement.span.end.offset))
    } catch (error) {
      toast.error('Could not read the SQL', { description: String(error) })
    }
  }, [])

  const showProblem = useCallback(() => {
    const current = useTabRuns.getState().runs[shownTab()]
    if (viewRef.current && current?.status === 'done' && current.problemAt) {
      revealPosition(viewRef.current, current.problemAt)
    }
  }, [])

  const save = useCallback(
    async (tabId: string, name: string, id?: string) => {
      const tab = useEditorTabs.getState().tabs.find((t) => t.id === tabId)
      if (!tab) return
      const saved = await saveQuery({ id, name, sql: tab.sql, connectionId })
      linkSaved(tabId, saved)
      toast.success(`Saved ${saved.name}`)
      await queryClient.invalidateQueries({ queryKey: [SAVED_KEY] })
    },
    [connectionId, linkSaved, queryClient],
  )

  /** Updates the linked saved query, or asks for a name for a new one. */
  const saveCurrent = useCallback(async () => {
    const tab = useEditorTabs.getState().tabs.find((t) => t.id === shownTab())
    if (!tab) return
    if (tab.sql.trim() === '') {
      toast('Nothing to save', { description: 'Write a query first.' })
      return
    }
    if (!tab.savedQueryId) {
      setSaveOpen(true)
      return
    }
    try {
      await save(tab.id, tab.title, tab.savedQueryId)
    } catch (error) {
      if (error instanceof NativeError && error.code === 'not_found') {
        // It was deleted from the library: save it as a new query.
        setSaveOpen(true)
        return
      }
      toast.error("Couldn't save the query", {
        description: error instanceof Error ? error.message : String(error),
      })
    }
  }, [save])

  const openSql = useCallback(
    (sql: string, saved?: SavedQuery) => {
      const state = useEditorTabs.getState()
      const linked = saved && state.tabs.find((tab) => tab.savedQueryId === saved.id)
      if (linked) {
        select(linked.id)
        return
      }
      if (state.tabs.length >= MAX_TABS) {
        toast('Too many tabs', { description: `Close a tab first (${MAX_TABS} at most).` })
        return
      }
      add(sql, saved && { id: saved.id, name: saved.name })
    },
    [add, select],
  )

  const cancel = useCallback(() => {
    const current = useTabRuns.getState().runs[shownTab()]
    if (current?.status !== 'running') return false
    cancelOperation(connectionId, current.operationId).catch(() => undefined)
    return true
  }, [connectionId])

  const closeTab = useCallback(
    (id: string) => {
      const tab = useEditorTabs.getState().tabs.find((t) => t.id === id)
      close(id)
      forgetRun(id)
      if (tab && tab.sql.trim() !== '') {
        toast(`Closed ${tab.title}`, {
          duration: 8_000,
          action: { label: 'Undo', onClick: () => useEditorTabs.getState().add(tab.sql) },
        })
      }
    },
    [close, forgetRun],
  )

  // Tab shortcuts: Ctrl/Cmd+Alt+N new, +W close, +←/→ previous and next.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!(event.altKey && (isMac ? event.metaKey : event.ctrlKey))) return
      const key = event.key.toLowerCase()
      const actions: Record<string, () => void> = {
        n: () => add(),
        s: () => void saveCurrent(),
        w: () => closeTab(shownTab()),
        arrowright: () => cycle(1),
        arrowleft: () => cycle(-1),
      }
      const action = actions[key] ?? actions[event.code.replace('Key', '').toLowerCase()]
      if (action) {
        event.preventDefault()
        action()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [add, closeTab, cycle, saveCurrent])

  // Tab keys (WAI-ARIA tabs pattern): arrows, Home and End move and select; Delete closes.
  const focusTab = (id: string | undefined) => {
    if (!id) return
    requestAnimationFrame(() =>
      tablistRef.current?.querySelector<HTMLElement>(`[data-tab-id="${id}"]`)?.focus(),
    )
  }
  const onTabKey = (event: ReactKeyboardEvent) => {
    const current = (event.target as HTMLElement).dataset.tabId
    if (!current) return
    const ids = tabs.map((tab) => tab.id)
    const at = ids.indexOf(current)
    const target = {
      ArrowRight: ids[(at + 1) % ids.length],
      ArrowLeft: ids[(at - 1 + ids.length) % ids.length],
      Home: ids[0],
      End: ids[ids.length - 1],
    }[event.key]
    if (target) {
      event.preventDefault()
      select(target)
      focusTab(target)
    } else if (event.key === 'Delete') {
      event.preventDefault()
      closeTab(current)
      focusTab(useEditorTabs.getState().activeId)
    }
  }

  if (!activeTab) return null

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 items-end gap-1 border-b px-2 pt-2">
        <div
          ref={tablistRef}
          role="tablist"
          aria-label="SQL drafts"
          className="flex min-w-0 items-end gap-1 overflow-x-auto"
          onKeyDown={onTabKey}
        >
          {tabs.map((tab) => {
            const selected = tab.id === activeId
            return (
              <div
                key={tab.id}
                className={cn(
                  'flex shrink-0 items-center rounded-t-md border border-b-0',
                  selected ? 'bg-background' : 'border-transparent bg-muted/50',
                )}
              >
                <button
                  type="button"
                  role="tab"
                  id={`sql-tab-${tab.id}`}
                  data-tab-id={tab.id}
                  aria-selected={selected}
                  aria-controls={selected ? 'sql-tab-panel' : undefined}
                  aria-keyshortcuts="Delete"
                  tabIndex={selected ? 0 : -1}
                  onClick={() => select(tab.id)}
                  className={cn(
                    'h-8 max-w-48 truncate px-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring max-sm:h-11',
                    selected ? 'font-medium' : 'text-muted-foreground',
                  )}
                >
                  {tab.title}
                  {runs[tab.id]?.status === 'running' && (
                    <span className="sr-only"> (running)</span>
                  )}
                </button>
                {/* A pointer shortcut only: a tablist may hold nothing but tabs, so keyboard and
                    screen reader users close the focused tab with Delete instead. */}
                <Button
                  variant="ghost"
                  size="icon-xs"
                  className="mr-1 max-sm:size-11"
                  aria-hidden
                  tabIndex={-1}
                  title={`Close ${tab.title}`}
                  onClick={() => closeTab(tab.id)}
                >
                  <XIcon />
                </Button>
              </div>
            )
          })}
        </div>
        <Button
          variant="ghost"
          size="icon-sm"
          className="mb-0.5 shrink-0 max-sm:size-11"
          aria-label="New tab"
          title={`New tab (${MOD}+Alt+N)`}
          disabled={tabs.length >= MAX_TABS}
          onClick={() => add()}
        >
          <PlusIcon />
        </Button>
      </div>

      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b px-4 py-2">
        {running ? (
          <Button variant="secondary" className="max-sm:min-h-11" onClick={cancel}>
            <SquareIcon data-icon="inline-start" />
            Cancel
          </Button>
        ) : (
          <Button className="max-sm:min-h-11" onClick={() => void runCurrent()}>
            <PlayIcon data-icon="inline-start" />
            Run
          </Button>
        )}
        <Button
          variant="outline"
          className="max-sm:min-h-11"
          disabled={running}
          title="Show the query's plan and what it suggests"
          onClick={() => void explainCurrent()}
        >
          <ListTreeIcon data-icon="inline-start" />
          Explain
        </Button>
        {running ? (
          <span className="flex items-center gap-2 text-sm text-muted-foreground" role="status">
            <Spinner />
            Running…
          </span>
        ) : (
          <span className="text-sm text-muted-foreground max-lg:hidden">
            Runs the selection, or the statement at the cursor. <Kbd>{MOD}</Kbd> <Kbd>Enter</Kbd> to
            run, <Kbd>Esc</Kbd> to cancel.
          </span>
        )}
        <span className="ml-auto text-xs text-muted-foreground max-2xl:hidden">
          Reads only, in a transaction that is rolled back.
        </span>
        <div className="flex items-center gap-1 max-2xl:ml-auto">
          <Button
            variant="outline"
            className="max-sm:min-h-11"
            title={`Save (${MOD}+Alt+S)`}
            onClick={() => void saveCurrent()}
          >
            <SaveIcon data-icon="inline-start" />
            Save
          </Button>
          <Button
            variant="outline"
            className="max-sm:min-h-11"
            onClick={() => setLibraryOpen(true)}
          >
            <LibraryBigIcon data-icon="inline-start" />
            Library
          </Button>
        </div>
      </div>

      <div
        id="sql-tab-panel"
        role="tabpanel"
        aria-labelledby={`sql-tab-${activeTab.id}`}
        className="h-[40%] min-h-36 shrink-0 border-b"
      >
        <SqlEditor
          tabId={activeTab.id}
          openTabs={tabs.map((tab) => tab.id)}
          initialDoc={activeTab.sql}
          schema={schema}
          onChange={update}
          onRun={() => void runCurrent()}
          onCancel={cancel}
          viewRef={viewRef}
        />
      </div>

      <PlanSheet connectionId={connectionId} sql={explaining} onClose={() => setExplaining(null)} />
      <section aria-label="Result" className="flex min-h-0 flex-1 flex-col">
        {doneRun ? (
          <QueryResultView
            key={activeId}
            outcome={doneRun.outcome}
            start={doneRun.start}
            problemAt={doneRun.problemAt}
            busy={running}
            pager={{
              page: doneRun.previous.length + 1,
              onPrevious:
                doneRun.outcome.status === 'completed' &&
                pageTrail(doneRun.outcome, doneRun.previous, 'previous')
                  ? () => void loadPage('previous')
                  : null,
              onNext:
                doneRun.outcome.status === 'completed' &&
                pageTrail(doneRun.outcome, doneRun.previous, 'next')
                  ? () => void loadPage('next')
                  : null,
            }}
            onShowProblem={showProblem}
          />
        ) : running ? null : (
          <div className="p-4">
            <EmptyState
              icon={SquareTerminalIcon}
              title="No result yet"
              description="Run a statement to see its rows here."
            />
          </div>
        )}
      </section>

      <LibrarySheet
        open={libraryOpen}
        onOpenChange={setLibraryOpen}
        connectionId={connectionId}
        onOpenSql={openSql}
      />
      <SaveQueryDialog
        open={saveOpen}
        onOpenChange={setSaveOpen}
        defaultName={activeTab.title}
        onSave={async (name) => {
          await save(activeTab.id, name)
          setSaveOpen(false)
        }}
      />
    </div>
  )
}
