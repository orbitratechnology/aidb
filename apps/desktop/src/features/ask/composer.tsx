import { useQuery } from '@tanstack/react-query'
import type { AskRequest, ConnectionId, ContextPreview } from '@aidb/contracts'
import { ASSISTANT_LIMITS } from '@aidb/contracts'
import {
  ArrowUpIcon,
  FileTextIcon,
  LayersIcon,
  PaperclipIcon,
  SquareIcon,
  TableIcon,
  XIcon,
} from 'lucide-react'
import { useEffect, useId, useRef, useState, type KeyboardEvent } from 'react'
import { toast } from 'sonner'
import {
  Attachment,
  AttachmentAction,
  AttachmentActions,
  AttachmentContent,
  AttachmentDescription,
  AttachmentGroup,
  AttachmentMedia,
  AttachmentTitle,
} from '@/components/ui/attachment'
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command'
import { Field, FieldContent, FieldDescription, FieldLabel } from '@/components/ui/field'
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupTextarea,
} from '@/components/ui/input-group'
import {
  Popover,
  PopoverContent,
  PopoverDescription,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
} from '@/components/ui/popover'
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { useCatalog } from '@/features/data/queries'
import { previewAssistantContext } from '@/lib/commands'
import { NativeError } from '@/lib/native'
import { useAsk } from './ask-store'
import {
  ATTACHMENT_ACCEPT,
  attachmentProblem,
  formatSize,
  historyFor,
  objectLabel,
} from './conversation'
import { formatTokens } from './display'
import { ContextItemList } from './turn-view'

const PREVIEW_DELAY_MS = 400

function useDebounced<T>(value: T, delay: number): T {
  const [debounced, setDebounced] = useState(value)
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value), delay)
    return () => clearTimeout(timer)
  }, [value, delay])
  return debounced
}

type ComposerProps = {
  connectionId: ConnectionId
  database: string
  provider: { name: string; hosted: boolean }
}

export function Composer({ connectionId, database, provider }: ComposerProps) {
  const draft = useAsk((state) => state.draft)
  const shareData = useAsk((state) => state.shareData)
  const running = useAsk((state) => state.runningTurn !== null)
  const turns = useAsk((state) => state.turns)
  const { setQuestion, send, stop, removeAttachment, toggleTable, setShareData } = useAsk.getState()
  const shareId = useId()
  const fileInput = useRef<HTMLInputElement>(null)
  const question = draft.question.trim()
  const tooLong = draft.question.length > ASSISTANT_LIMITS.questionMaxChars
  const filesNeedPermission = provider.hosted && !shareData

  // What would be sent, refreshed shortly after the draft changes. Nothing is sent to the model.
  const request: AskRequest = {
    question: useDebounced(question, PREVIEW_DELAY_MS),
    selected: draft.selected,
    attachments: draft.attachments.map(({ name, content }) => ({ name, content })),
    history: historyFor(turns),
    shareData,
  }
  const preview = useQuery({
    queryKey: [
      'assistant-preview',
      connectionId,
      request.question,
      draft.selected,
      draft.attachments.map((file) => file.id),
      request.history.length,
      shareData,
    ],
    queryFn: () => previewAssistantContext(connectionId, request),
    enabled: request.question !== '' && !tooLong,
    retry: false,
  })

  const previewError = preview.error instanceof Error ? preview.error.message : null
  // The core would refuse it the same way: fix the request first.
  const refused =
    preview.error instanceof NativeError &&
    (preview.error.code === 'invalid_input' || preview.error.code === 'unavailable')
  const canSend = question !== '' && !tooLong && !running && !refused

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault()
      if (canSend) void send()
    }
  }

  const onFiles = async (files: FileList | null) => {
    for (const file of Array.from(files ?? [])) {
      const problem = attachmentProblem(file, useAsk.getState().draft.attachments)
      if (problem) {
        toast.error('File not attached', { description: problem })
        continue
      }
      try {
        const content = await file.text()
        useAsk
          .getState()
          .addAttachment({ id: crypto.randomUUID(), name: file.name, size: file.size, content })
      } catch {
        toast.error('File not attached', { description: `“${file.name}” could not be read.` })
      }
    }
    if (fileInput.current) fileInput.current.value = ''
  }

  return (
    <form
      className="flex flex-col gap-2"
      onSubmit={(event) => {
        event.preventDefault()
        if (canSend) void send()
      }}
    >
      {(draft.attachments.length > 0 || draft.selected.length > 0) && (
        <AttachmentGroup aria-label="Added to the question">
          {draft.selected.map((object) => (
            <Attachment key={objectLabel(object)} size="xs">
              <AttachmentMedia variant="icon">
                <TableIcon />
              </AttachmentMedia>
              <AttachmentContent>
                <AttachmentTitle>{objectLabel(object)}</AttachmentTitle>
                <AttachmentDescription>Table described in full</AttachmentDescription>
              </AttachmentContent>
              <AttachmentActions>
                <AttachmentAction
                  aria-label={`Remove ${objectLabel(object)}`}
                  onClick={() => toggleTable(object)}
                >
                  <XIcon />
                </AttachmentAction>
              </AttachmentActions>
            </Attachment>
          ))}
          {draft.attachments.map((file) => (
            <Attachment key={file.id} size="xs">
              <AttachmentMedia variant="icon">
                <FileTextIcon />
              </AttachmentMedia>
              <AttachmentContent>
                <AttachmentTitle>{file.name}</AttachmentTitle>
                <AttachmentDescription>{formatSize(file.size)}</AttachmentDescription>
              </AttachmentContent>
              <AttachmentActions>
                <AttachmentAction
                  aria-label={`Remove ${file.name}`}
                  onClick={() => removeAttachment(file.id)}
                >
                  <XIcon />
                </AttachmentAction>
              </AttachmentActions>
            </Attachment>
          ))}
        </AttachmentGroup>
      )}

      <InputGroup>
        <InputGroupTextarea
          aria-label="Question"
          placeholder={`Ask about ${database}…`}
          value={draft.question}
          onChange={(event) => setQuestion(event.target.value)}
          onKeyDown={onKeyDown}
          aria-invalid={tooLong || undefined}
          className="max-h-48 min-h-16"
        />
        <InputGroupAddon align="block-end" className="flex-wrap">
          <input
            ref={fileInput}
            type="file"
            accept={ATTACHMENT_ACCEPT}
            multiple
            hidden
            onChange={(event) => void onFiles(event.target.files)}
          />
          <Tooltip>
            <TooltipTrigger asChild>
              {/* A disabled button gets no pointer events; the wrapper keeps the hint reachable. */}
              <span tabIndex={filesNeedPermission ? 0 : -1}>
                <InputGroupButton
                  variant="ghost"
                  size="sm"
                  className="max-sm:min-h-11"
                  disabled={filesNeedPermission}
                  onClick={() => fileInput.current?.click()}
                >
                  <PaperclipIcon data-icon="inline-start" />
                  Attach
                </InputGroupButton>
              </span>
            </TooltipTrigger>
            <TooltipContent>
              {filesNeedPermission
                ? `Allow sharing data with ${provider.name} to attach files.`
                : '.sql, .csv, .json, .md or .txt files'}
            </TooltipContent>
          </Tooltip>
          <TablePicker connectionId={connectionId} />
          <ContextReview preview={preview.data} loading={preview.isFetching} error={previewError} />
          {running ? (
            <InputGroupButton
              variant="outline"
              size="sm"
              className="ml-auto max-sm:min-h-11"
              onClick={() => void stop()}
            >
              <SquareIcon data-icon="inline-start" />
              Stop
            </InputGroupButton>
          ) : (
            <InputGroupButton
              type="submit"
              variant="default"
              size="sm"
              className="ml-auto max-sm:min-h-11"
              disabled={!canSend}
            >
              <ArrowUpIcon data-icon="inline-start" />
              Ask
            </InputGroupButton>
          )}
        </InputGroupAddon>
      </InputGroup>

      {tooLong && (
        <p role="alert" className="text-sm text-destructive">
          The question is too long ({ASSISTANT_LIMITS.questionMaxChars.toLocaleString()} characters
          at most).
        </p>
      )}
      {!tooLong && previewError && (
        <p role="alert" className="text-sm text-destructive">
          {previewError}
        </p>
      )}

      {provider.hosted && (
        <Field orientation="horizontal">
          <Switch id={shareId} checked={shareData} onCheckedChange={setShareData} />
          <FieldContent>
            <FieldLabel htmlFor={shareId}>
              Share query results and files with {provider.name}
            </FieldLabel>
            <FieldDescription>
              For this conversation only. Without it, the assistant sees the database structure but
              no row values, and cannot read attached files.
            </FieldDescription>
          </FieldContent>
        </Field>
      )}
    </form>
  )
}

/** Picks tables to describe in full; the rest of the schema is summarized. */
function TablePicker({ connectionId }: { connectionId: ConnectionId }) {
  const catalog = useCatalog(connectionId)
  const selected = useAsk((state) => state.draft.selected)
  const toggleTable = useAsk((state) => state.toggleTable)
  const full = selected.length >= ASSISTANT_LIMITS.maxSelectedTables
  return (
    <Popover>
      <PopoverTrigger asChild>
        <InputGroupButton variant="ghost" size="sm" className="max-sm:min-h-11">
          <TableIcon data-icon="inline-start" />
          Tables{selected.length > 0 ? ` (${selected.length})` : ''}
        </InputGroupButton>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-80 p-0">
        <Command>
          <CommandInput placeholder="Find a table or view" />
          <CommandList>
            <CommandEmpty>
              {catalog.isPending ? 'Loading tables…' : 'No table or view matches.'}
            </CommandEmpty>
            {catalog.data?.schemas.map((schema) => {
              const objects = catalog.data.objects.filter((object) => object.schema === schema.name)
              if (objects.length === 0) return null
              return (
                <CommandGroup key={schema.name} heading={schema.name}>
                  {objects.map((object) => {
                    const isSelected = selected.some(
                      (other) => other.schema === object.schema && other.name === object.name,
                    )
                    return (
                      <CommandItem
                        key={`${object.schema}.${object.name}`}
                        value={`${object.schema}.${object.name}`}
                        data-checked={isSelected}
                        disabled={!isSelected && full}
                        onSelect={() => toggleTable({ schema: object.schema, name: object.name })}
                      >
                        <TableIcon />
                        <span className="truncate">{object.name}</span>
                        <span className="sr-only">{isSelected ? ', selected' : ''}</span>
                      </CommandItem>
                    )
                  })}
                </CommandGroup>
              )
            })}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}

/** The list of what would be sent, reviewed before asking. */
function ContextReview({
  preview,
  loading,
  error,
}: {
  preview: ContextPreview | undefined
  loading: boolean
  error: string | null
}) {
  return (
    <Popover>
      <PopoverTrigger asChild>
        <InputGroupButton
          variant="ghost"
          size="sm"
          className="max-sm:min-h-11"
          disabled={!preview && !loading}
        >
          {loading ? <Spinner data-icon="inline-start" /> : <LayersIcon data-icon="inline-start" />}
          {preview ? formatTokens(preview.estimatedTokens) : 'Context'}
        </InputGroupButton>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-96 max-w-[calc(100vw-2rem)]">
        <PopoverHeader>
          <PopoverTitle>What will be sent</PopoverTitle>
          <PopoverDescription>
            {preview
              ? `To ${preview.provider.name} (${preview.provider.model}), ${formatTokens(preview.estimatedTokens)} of ${preview.budgetTokens.toLocaleString()}. ${
                  preview.dataAllowed
                    ? 'Query results may be sent too.'
                    : 'Query results are not sent.'
                } Passwords and API keys never are.`
              : (error ?? 'Write a question to see what will be sent.')}
          </PopoverDescription>
        </PopoverHeader>
        {preview && <ContextItemList items={preview.items} />}
      </PopoverContent>
    </Popover>
  )
}
