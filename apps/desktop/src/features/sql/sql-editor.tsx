import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from 'react'
import {
  autocompletion,
  closeBrackets,
  closeBracketsKeymap,
  completionKeymap,
} from '@codemirror/autocomplete'
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands'
import { PostgreSQL, sql, type SQLNamespace } from '@codemirror/lang-sql'
import {
  bracketMatching,
  HighlightStyle,
  indentOnInput,
  syntaxHighlighting,
} from '@codemirror/language'
import { lintKeymap } from '@codemirror/lint'
import { highlightSelectionMatches, searchKeymap } from '@codemirror/search'
import { Compartment, EditorState, Prec, type Extension } from '@codemirror/state'
import {
  drawSelection,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
  placeholder,
} from '@codemirror/view'
import { tags } from '@lezer/highlight'

/** Colors come from the app's theme tokens, so light and dark modes follow automatically. */
const theme = EditorView.theme({
  '&': {
    height: '100%',
    color: 'var(--foreground)',
    backgroundColor: 'var(--background)',
    fontSize: '0.875rem',
  },
  '&.cm-focused': { outline: 'none' },
  '.cm-scroller': { fontFamily: 'var(--font-mono)', lineHeight: '1.6' },
  '.cm-content': { caretColor: 'var(--foreground)', padding: '0.5rem 0' },
  '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--foreground)' },
  '.cm-gutters': {
    backgroundColor: 'var(--background)',
    color: 'var(--muted-foreground)',
    borderRight: '1px solid var(--border)',
  },
  '.cm-activeLine': { backgroundColor: 'color-mix(in oklab, var(--muted) 55%, transparent)' },
  '.cm-activeLineGutter': { backgroundColor: 'var(--muted)', color: 'var(--foreground)' },
  '&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection':
    { backgroundColor: 'color-mix(in oklab, var(--ring) 40%, transparent)' },
  '.cm-selectionMatch': { backgroundColor: 'color-mix(in oklab, var(--ring) 20%, transparent)' },
  '.cm-matchingBracket': {
    backgroundColor: 'color-mix(in oklab, var(--ring) 30%, transparent)',
    outline: 'none',
  },
  '.cm-tooltip': {
    backgroundColor: 'var(--popover)',
    color: 'var(--popover-foreground)',
    border: '1px solid var(--border)',
    borderRadius: 'calc(var(--radius) - 2px)',
  },
  '.cm-tooltip-autocomplete ul li[aria-selected]': {
    backgroundColor: 'var(--accent)',
    color: 'var(--accent-foreground)',
  },
  '.cm-diagnostic': { padding: '0.375rem 0.625rem' },
  '.cm-diagnostic-error': { borderLeft: '3px solid var(--destructive)' },
  '.cm-lintRange-error': {
    backgroundImage: 'none',
    textDecoration: 'underline wavy var(--destructive)',
    textUnderlineOffset: '3px',
  },
  '.cm-panels': {
    backgroundColor: 'var(--muted)',
    color: 'var(--foreground)',
    borderColor: 'var(--border)',
  },
  '.cm-textfield': {
    backgroundColor: 'var(--background)',
    border: '1px solid var(--input)',
    borderRadius: 'calc(var(--radius) - 4px)',
  },
  '.cm-button': {
    backgroundImage: 'none',
    backgroundColor: 'var(--background)',
    border: '1px solid var(--border)',
    borderRadius: 'calc(var(--radius) - 4px)',
  },
})

const highlighting = HighlightStyle.define([
  {
    tag: [tags.keyword, tags.operatorKeyword, tags.modifier],
    color: 'var(--syntax-keyword)',
    fontWeight: '600',
  },
  { tag: [tags.string, tags.special(tags.string)], color: 'var(--syntax-string)' },
  { tag: [tags.number, tags.bool, tags.null], color: 'var(--syntax-number)' },
  { tag: [tags.typeName, tags.standard(tags.name)], color: 'var(--syntax-type)' },
  {
    tag: [tags.comment, tags.lineComment, tags.blockComment],
    color: 'var(--muted-foreground)',
    fontStyle: 'italic',
  },
  { tag: tags.special(tags.name), color: 'var(--syntax-string)' },
])

type Props = {
  tabId: string
  /** Every open tab, so the states of closed ones can be dropped. */
  openTabs: string[]
  /** The draft to start from when a tab is first shown. */
  initialDoc: string
  /** Tables to complete, from the schema catalog. */
  schema: SQLNamespace
  onChange: (tabId: string, doc: string) => void
  /** Ctrl/Cmd+Enter. */
  onRun: () => void
  /** Escape; returns whether a running query was cancelled. */
  onCancel: () => boolean
  /** Set to the live editor, for the toolbar's Run button and diagnostics. */
  viewRef: RefObject<EditorView | null>
}

/**
 * CodeMirror 6 with PostgreSQL highlighting and completion. One editor serves every tab: each
 * tab keeps its own state (text, selection, undo history, diagnostics) while others are shown.
 */
export function SqlEditor({
  tabId,
  openTabs,
  initialDoc,
  schema,
  onChange,
  onRun,
  onCancel,
  viewRef,
}: Props) {
  const host = useRef<HTMLDivElement>(null)
  const states = useRef(new Map<string, EditorState>())
  const shownTab = useRef(tabId)
  const extensions = useRef<Extension[]>([])
  const [language] = useState(() => new Compartment())
  const latest = useRef({ onChange, onRun, onCancel, schema, initialDoc })
  // Event handlers inside the editor read the latest props from here.
  useLayoutEffect(() => {
    latest.current = { onChange, onRun, onCancel, schema, initialDoc }
  })

  // Create the editor once; destroy it on unmount.
  useEffect(() => {
    if (!host.current) return
    extensions.current = [
      lineNumbers(),
      highlightActiveLineGutter(),
      history(),
      drawSelection(),
      indentOnInput(),
      bracketMatching(),
      closeBrackets(),
      autocompletion(),
      highlightActiveLine(),
      highlightSelectionMatches(),
      syntaxHighlighting(highlighting),
      language.of(sqlLanguage(latest.current.schema)),
      theme,
      placeholder('Write SQL here. Ctrl+Enter runs the selection, or the statement at the cursor.'),
      EditorView.contentAttributes.of({ 'aria-label': 'SQL editor' }),
      EditorView.lineWrapping,
      Prec.highest(
        keymap.of([
          {
            key: 'Mod-Enter',
            run: () => {
              latest.current.onRun()
              return true
            },
          },
          { key: 'Escape', run: () => latest.current.onCancel() },
        ]),
      ),
      keymap.of([
        ...closeBracketsKeymap,
        ...defaultKeymap,
        ...searchKeymap,
        ...historyKeymap,
        ...completionKeymap,
        ...lintKeymap,
      ]),
      EditorView.updateListener.of((update) => {
        if (update.docChanged) {
          latest.current.onChange(shownTab.current, update.state.doc.toString())
        }
      }),
    ]
    const view = new EditorView({
      parent: host.current,
      state: EditorState.create({
        doc: latest.current.initialDoc,
        extensions: extensions.current,
      }),
    })
    viewRef.current = view
    return () => {
      view.destroy()
      viewRef.current = null
    }
  }, [language, viewRef])

  // Swap in the shown tab's state, keeping the previous tab's for later.
  useEffect(() => {
    const view = viewRef.current
    if (!view || shownTab.current === tabId) return
    states.current.set(shownTab.current, view.state)
    shownTab.current = tabId
    const saved = states.current.get(tabId)
    view.setState(
      saved ??
        EditorState.create({ doc: latest.current.initialDoc, extensions: extensions.current }),
    )
    view.dispatch({ effects: language.reconfigure(sqlLanguage(latest.current.schema)) })
    view.focus()
  }, [tabId, language, viewRef])

  // Forget closed tabs.
  useEffect(() => {
    for (const id of states.current.keys()) {
      if (!openTabs.includes(id)) states.current.delete(id)
    }
  }, [openTabs])

  // Completion follows the schema catalog.
  useEffect(() => {
    viewRef.current?.dispatch({ effects: language.reconfigure(sqlLanguage(schema)) })
  }, [schema, language, viewRef])

  return <div ref={host} className="h-full min-h-0 overflow-hidden" />
}

function sqlLanguage(schema: SQLNamespace) {
  return sql({ dialect: PostgreSQL, schema, defaultSchema: 'public' })
}
