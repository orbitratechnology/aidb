import { useCallback, useEffect, useMemo, useRef } from 'react'
import type { CatalogSnapshot, ConnectionId, QueryRun } from '@aidb/contracts'
import type { SQLNamespace } from '@codemirror/lang-sql'
import { setDiagnostics } from '@codemirror/lint'
import type { EditorView } from '@codemirror/view'
import { PlayIcon, PlusIcon, SquareIcon, SquareTerminalIcon, XIcon } from 'lucide-react'
import { toast } from 'sonner'
import { PageContainer } from '@/components/shared/page'
import { ConnectionRequired, EmptyState } from '@/components/shared/status-states'
import { Button } from '@/components/ui/button'
import { Kbd } from '@/components/ui/kbd'
import { Spinner } from '@/components/ui/spinner'
import { useActiveConnection } from '@/features/connections/sessions'
import { useCatalog } from '@/features/data/queries'
import { analyzeSql, cancelOperation, runQuery } from '@/lib/commands'
import { cn } from '@/lib/utils'
import { useEditorTabs } from './editor-store'
import { QueryResultView } from './query-result-view'
import { useTabRuns } from './run-store'
import { docPosition, problemRange, statementAt } from './run-target'
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

function SqlWorkspace({ connectionId }: { connectionId: ConnectionId }) {
  const { tabs, activeId, add, close, select, cycle, update } = useEditorTabs()
  const runs = useTabRuns((state) => state.runs)
  const setRun = useTabRuns((state) => state.set)
  const forgetRun = useTabRuns((state) => state.forget)
  const viewRef = useRef<EditorView | null>(null)
  const catalog = useCatalog(connectionId)
  const schema = useMemo(() => namespace(catalog.data), [catalog.data])
  const activeTab = tabs.find((tab) => tab.id === activeId) ?? tabs[0]
  const run = runs[activeId]
  // Results belong to the connection they ran on.
  const shownRun = run && run.connectionId === connectionId ? run : undefined
  const running = shownRun?.status === 'running'

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
        finish(view, tabId, doc, 0, {
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
      finish(view, tabId, doc, base, outcome)
    } catch (error) {
      forgetRun(tabId)
      toast.error('Could not run the statement', {
        description: error instanceof Error ? error.message : String(error),
      })
    }

    function finish(view: EditorView, tabId: string, doc: string, base: number, outcome: QueryRun) {
      const problem = outcome.status === 'completed' ? null : outcome
      setRun(tabId, {
        status: 'done',
        connectionId,
        outcome,
        start: docPosition(doc, base),
        problemAt: problem?.position ? docPosition(doc, base + problem.position.offset) : null,
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
  }, [connectionId, forgetRun, setRun])

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
  }, [add, closeTab, cycle])

  if (!activeTab) return null

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div
        role="tablist"
        aria-label="SQL drafts"
        className="flex shrink-0 items-end gap-1 overflow-x-auto border-b px-2 pt-2"
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
                aria-selected={selected}
                tabIndex={selected ? 0 : -1}
                onClick={() => select(tab.id)}
                className={cn(
                  'h-8 max-w-48 truncate px-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring max-sm:h-11',
                  selected ? 'font-medium' : 'text-muted-foreground',
                )}
              >
                {tab.title}
                {runs[tab.id]?.status === 'running' && <span className="sr-only"> (running)</span>}
              </button>
              <Button
                variant="ghost"
                size="icon-xs"
                className="mr-1 max-sm:size-9"
                aria-label={`Close ${tab.title}`}
                onClick={() => closeTab(tab.id)}
              >
                <XIcon />
              </Button>
            </div>
          )
        })}
        <Button
          variant="ghost"
          size="icon-sm"
          className="mb-0.5 max-sm:size-11"
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
        {running ? (
          <span className="flex items-center gap-2 text-sm text-muted-foreground" role="status">
            <Spinner />
            Running…
          </span>
        ) : (
          <span className="text-sm text-muted-foreground max-sm:hidden">
            Runs the selection, or the statement at the cursor. <Kbd>{MOD}</Kbd> <Kbd>Enter</Kbd> to
            run, <Kbd>Esc</Kbd> to cancel.
          </span>
        )}
        <span className="ml-auto text-xs text-muted-foreground max-md:hidden">
          Reads only, in a transaction that is rolled back.
        </span>
      </div>

      <div className="h-[40%] min-h-36 shrink-0 border-b">
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

      <section aria-label="Result" className="flex min-h-0 flex-1 flex-col">
        {shownRun?.status === 'done' ? (
          <QueryResultView
            key={activeId}
            outcome={shownRun.outcome}
            start={shownRun.start}
            problemAt={shownRun.problemAt}
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
    </div>
  )
}
