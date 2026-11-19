import { SquareTerminalIcon } from 'lucide-react'
import { useNavigate } from 'react-router'
import { Button } from '@/components/ui/button'
import {
  CodeBlock,
  CodeBlockActions,
  CodeBlockContent,
  CodeBlockCopyButton,
  CodeBlockHeader,
  CodeBlockLine,
  CodeBlockTitle,
} from '@/components/ui/code-block'
import { useHighlightedCode } from '@/hooks/use-highlighted-code'
import { cn } from '@/lib/utils'
import { useEditorTabs } from '@/features/sql/editor-store'
import { isSql, splitAnswer, type AnswerPart } from './answer'

/**
 * Model text: prose as plain text (never HTML or Markdown rendering), fenced code in code
 * blocks. SQL can be copied or opened in a new editor tab; nothing runs from here.
 */
export function AnswerText({
  text,
  muted = false,
  streaming = false,
}: {
  text: string
  muted?: boolean
  streaming?: boolean
}) {
  const parts = splitAnswer(text, streaming)
  return (
    <div className={cn('flex min-w-0 flex-col gap-3', muted && 'text-muted-foreground')}>
      {parts.map((part, index) =>
        part.kind === 'text' ? (
          <p key={index} className="leading-relaxed wrap-break-word whitespace-pre-wrap">
            {part.text}
          </p>
        ) : (
          <AnswerCode key={index} part={part} />
        ),
      )}
    </div>
  )
}

function AnswerCode({ part }: { part: Extract<AnswerPart, { kind: 'code' }> }) {
  const lines = useHighlightedCode(part.code, part.language || 'text')
  const navigate = useNavigate()
  const sql = isSql(part)
  return (
    <CodeBlock>
      <CodeBlockHeader>
        <CodeBlockTitle>{sql ? 'SQL' : part.language || 'Code'}</CodeBlockTitle>
        <CodeBlockActions>
          {sql && part.complete && (
            <Button
              variant="ghost"
              size="xs"
              onClick={() => {
                useEditorTabs.getState().add(part.code)
                void navigate('/sql')
              }}
            >
              <SquareTerminalIcon data-icon="inline-start" />
              Open in SQL editor
            </Button>
          )}
          <CodeBlockCopyButton value={part.code} />
        </CodeBlockActions>
      </CodeBlockHeader>
      <CodeBlockContent wrap className="max-h-96">
        {lines.map((line, index) => (
          <CodeBlockLine key={index}>{line}</CodeBlockLine>
        ))}
      </CodeBlockContent>
    </CodeBlock>
  )
}
