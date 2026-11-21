/** A piece of an answer: prose, or a fenced code block. */
export type AnswerPart =
  | { kind: 'text'; text: string }
  | { kind: 'code'; language: string; code: string; complete: boolean }

const FENCE = /^ {0,3}(`{3,}|~{3,})\s*([\w+-]*)[^\n]*$/

/**
 * Splits model text into prose and fenced code blocks. Nothing is interpreted as HTML: prose is
 * shown as text. A fence still open at the end is a code part, incomplete while the text is
 * still `streaming` (models sometimes never close the last fence).
 */
export function splitAnswer(text: string, streaming = false): AnswerPart[] {
  const parts: AnswerPart[] = []
  const lines = text.split('\n')
  let prose: string[] = []
  let code: { fence: string; language: string; lines: string[] } | null = null

  const flushProse = () => {
    const joined = prose.join('\n').trim()
    if (joined) parts.push({ kind: 'text', text: joined })
    prose = []
  }

  for (const line of lines) {
    if (code) {
      const closing = line.trim()
      if (
        closing.startsWith(code.fence[0] ?? '`') &&
        closing.length >= code.fence.length &&
        /^(`+|~+)$/.test(closing)
      ) {
        parts.push({
          kind: 'code',
          language: code.language,
          code: code.lines.join('\n'),
          complete: true,
        })
        code = null
      } else {
        code.lines.push(line)
      }
      continue
    }
    const open = FENCE.exec(line)
    if (open) {
      flushProse()
      code = { fence: open[1] ?? '```', language: (open[2] ?? '').toLowerCase(), lines: [] }
    } else {
      prose.push(line)
    }
  }
  if (code) {
    parts.push({
      kind: 'code',
      language: code.language,
      code: code.lines.join('\n'),
      complete: !streaming,
    })
  }
  flushProse()
  return parts
}

/** Whether a code part holds SQL that may be opened in the editor. */
export function isSql(part: AnswerPart): boolean {
  return part.kind === 'code' && ['sql', 'pgsql', 'postgresql', 'postgres'].includes(part.language)
}
