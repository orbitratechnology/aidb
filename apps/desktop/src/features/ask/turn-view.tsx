import type { ContextItem, PlanStep, VerificationStep } from '@aidb/contracts'
import {
  CircleAlertIcon,
  CircleCheckIcon,
  CircleXIcon,
  FileTextIcon,
  LayersIcon,
  ListChecksIcon,
  SparklesIcon,
  TableIcon,
} from 'lucide-react'
import type { ReactNode } from 'react'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import {
  Attachment,
  AttachmentContent,
  AttachmentDescription,
  AttachmentGroup,
  AttachmentMedia,
  AttachmentTitle,
} from '@/components/ui/attachment'
import { Badge } from '@/components/ui/badge'
import { Bubble, BubbleContent } from '@/components/ui/bubble'
import { CodeBlock, CodeBlockContent } from '@/components/ui/code-block'
import { Message, MessageContent, MessageHeader } from '@/components/ui/message'
import {
  Trace,
  TraceCollapsible,
  TraceCollapsibleContent,
  TraceCollapsibleTrigger,
  TraceDescription,
  TraceIcon,
  TraceLine,
  TraceTitle,
} from '@/components/ui/trace'
import { streamingText, traceEntries, type ActivityEntry } from './activity'
import { AnswerText } from './answer-view'
import { formatSize, objectLabel, type Turn } from './conversation'
import {
  REJECTED_ICON,
  STEP_STATUS_LABELS,
  TASK_STATUS_DISPLAY,
  TOOL_ICONS,
  stepSql,
  traceStatus,
} from './display'

/** The person's question, with the files and tables they added. */
export function QuestionView({ turn }: { turn: Turn }) {
  const hasExtras = turn.attachments.length > 0 || turn.selected.length > 0
  return (
    <Message align="end">
      <MessageContent>
        <Bubble align="end">
          <BubbleContent className="whitespace-pre-wrap">{turn.question}</BubbleContent>
        </Bubble>
        {hasExtras && (
          <AttachmentGroup>
            {turn.selected.map((object) => (
              <Attachment key={objectLabel(object)} size="xs">
                <AttachmentMedia variant="icon">
                  <TableIcon />
                </AttachmentMedia>
                <AttachmentContent>
                  <AttachmentTitle>{objectLabel(object)}</AttachmentTitle>
                </AttachmentContent>
              </Attachment>
            ))}
            {turn.attachments.map((file) => (
              <Attachment key={file.name} size="xs">
                <AttachmentMedia variant="icon">
                  <FileTextIcon />
                </AttachmentMedia>
                <AttachmentContent>
                  <AttachmentTitle>{file.name}</AttachmentTitle>
                  <AttachmentDescription>{formatSize(file.size)}</AttachmentDescription>
                </AttachmentContent>
              </Attachment>
            ))}
          </AttachmentGroup>
        )}
      </MessageContent>
    </Message>
  )
}

/** What the assistant did and answered for one question. */
export function ReplyView({ turn, providerName }: { turn: Turn; providerName: string }) {
  const { activity, outcome } = turn
  const running = turn.state === 'running'
  const answer = outcome?.answer ?? null
  const entries = traceEntries(activity, running ? null : answer)
  const live = running ? streamingText(activity) : ''
  // While the model writes its reply, that text is shown as the answer, not in the trace.
  const shownEntries = live ? entries.slice(0, -1) : entries

  return (
    <Message align="start">
      <MessageContent>
        <MessageHeader>{providerName}</MessageHeader>
        {activity.items.length > 0 && <ContextSent items={activity.items} />}
        <ActivityRuns entries={shownEntries} />
        {running && !live && (
          <Trace>
            <TraceLine status="running">
              <TraceIcon>
                <SparklesIcon />
              </TraceIcon>
              <TraceTitle>{activity.taskId ? 'Working' : 'Preparing the question'}</TraceTitle>
            </TraceLine>
          </Trace>
        )}
        {live && <AnswerText text={live} streaming />}
        {!running && answer && <AnswerText text={answer} />}
        {turn.state === 'error' && (
          <Alert variant="destructive">
            <CircleXIcon />
            <AlertTitle>The question was not sent</AlertTitle>
            <AlertDescription>{turn.error}</AlertDescription>
          </Alert>
        )}
        {outcome && <OutcomeView outcome={outcome} />}
      </MessageContent>
    </Message>
  )
}

/** Notes break the trace into runs: one `Trace` per run of steps. */
function ActivityRuns({ entries }: { entries: ActivityEntry[] }) {
  const blocks: ReactNode[] = []
  let run: ActivityEntry[] = []
  const flush = () => {
    if (run.length === 0) return
    const lines = run
    blocks.push(
      <Trace key={`run-${blocks.length}`}>
        {lines.map((entry, index) =>
          entry.kind === 'step' ? (
            <StepLine key={`step-${entry.index}`} step={entry.step} />
          ) : entry.kind === 'rejected' ? (
            <RejectedLine key={`rejected-${index}`} tool={entry.tool} reason={entry.reason} />
          ) : null,
        )}
      </Trace>,
    )
    run = []
  }
  for (const entry of entries) {
    if (entry.kind === 'note') {
      flush()
      blocks.push(<AnswerText key={`note-${blocks.length}`} text={entry.text} muted />)
    } else {
      run.push(entry)
    }
  }
  flush()
  return blocks
}

function StepLine({ step }: { step: PlanStep }) {
  const Icon = TOOL_ICONS[step.tool]
  const sql = stepSql(step)
  const line = (
    <TraceLine status={traceStatus(step.status)}>
      <TraceIcon>
        <Icon />
      </TraceIcon>
      <TraceTitle>{step.summary}</TraceTitle>
      <TraceDescription>
        {step.status === 'succeeded' && step.outcome
          ? step.outcome
          : STEP_STATUS_LABELS[step.status]}
      </TraceDescription>
    </TraceLine>
  )
  const detail = step.status === 'succeeded' ? null : step.outcome
  if (!sql && !detail) return line
  return (
    <TraceCollapsible>
      <TraceCollapsibleTrigger>{line}</TraceCollapsibleTrigger>
      <TraceCollapsibleContent className="flex flex-col gap-2">
        {detail && <p className="wrap-break-word whitespace-pre-wrap">{detail}</p>}
        {sql && (
          <CodeBlock size="sm">
            <CodeBlockContent wrap className="max-h-64">
              {sql}
            </CodeBlockContent>
          </CodeBlock>
        )}
      </TraceCollapsibleContent>
    </TraceCollapsible>
  )
}

function RejectedLine({ tool, reason }: { tool: string; reason: string }) {
  return (
    <TraceCollapsible>
      <TraceCollapsibleTrigger>
        <TraceLine status="error">
          <TraceIcon>
            <REJECTED_ICON />
          </TraceIcon>
          <TraceTitle>Refused a request for “{tool}”</TraceTitle>
        </TraceLine>
      </TraceCollapsibleTrigger>
      <TraceCollapsibleContent>{reason}</TraceCollapsibleContent>
    </TraceCollapsible>
  )
}

const ITEM_ICONS: Record<ContextItem['kind'], typeof TableIcon> = {
  question: SparklesIcon,
  connection: LayersIcon,
  schema_overview: LayersIcon,
  selected_table: TableIcon,
  attachment: FileTextIcon,
}

/** The context that was sent with the question, for later review. */
function ContextSent({ items }: { items: ContextItem[] }) {
  return (
    <Trace>
      <TraceCollapsible>
        <TraceCollapsibleTrigger>
          <TraceLine>
            <TraceIcon>
              <LayersIcon />
            </TraceIcon>
            <TraceTitle>Sent {items.length} items of context</TraceTitle>
          </TraceLine>
        </TraceCollapsibleTrigger>
        <TraceCollapsibleContent>
          <ContextItemList items={items} />
        </TraceCollapsibleContent>
      </TraceCollapsible>
    </Trace>
  )
}

/** Context items with what was cut and what carries data values, in words. */
export function ContextItemList({ items }: { items: ContextItem[] }) {
  return (
    <Trace>
      {items.map((item, index) => {
        const Icon = ITEM_ICONS[item.kind]
        return (
          <TraceLine key={index}>
            <TraceIcon>
              <Icon />
            </TraceIcon>
            <TraceTitle>{item.label}</TraceTitle>
            <TraceDescription>
              {item.totalChars === 0
                ? 'in the instructions'
                : item.truncated
                  ? `cut to ${item.includedChars.toLocaleString()} of ${item.totalChars.toLocaleString()} characters`
                  : `${item.totalChars.toLocaleString()} characters`}
              {item.containsDataValues ? ' · contains data' : ''}
            </TraceDescription>
          </TraceLine>
        )
      })}
    </Trace>
  )
}

function OutcomeView({ outcome }: { outcome: NonNullable<Turn['outcome']> }) {
  const display =
    outcome.status === 'executing' || outcome.status === 'verifying'
      ? null
      : TASK_STATUS_DISPLAY[outcome.status]
  const checks = outcome.plan.verification
  const passed = checks.filter((check) => check.status === 'passed').length
  return (
    <div className="flex flex-col gap-2">
      {outcome.notice && (
        <Alert>
          <CircleAlertIcon />
          <AlertDescription>{outcome.notice}</AlertDescription>
        </Alert>
      )}
      <div className="flex flex-wrap items-center gap-2">
        {display && (
          <Badge variant={display.variant} title={display.description}>
            <display.icon data-icon="inline-start" />
            {display.label}
          </Badge>
        )}
        {checks.length > 0 && (
          <Trace className="w-auto">
            <TraceCollapsible>
              <TraceCollapsibleTrigger>
                <TraceLine status={passed < checks.length ? 'error' : undefined}>
                  <TraceIcon>
                    <ListChecksIcon />
                  </TraceIcon>
                  <TraceTitle>
                    {passed} of {checks.length} checks passed
                  </TraceTitle>
                </TraceLine>
              </TraceCollapsibleTrigger>
              <TraceCollapsibleContent>
                <Trace>
                  {checks.map((check, index) => (
                    <CheckLine key={index} check={check} />
                  ))}
                </Trace>
              </TraceCollapsibleContent>
            </TraceCollapsible>
          </Trace>
        )}
      </div>
    </div>
  )
}

function CheckLine({ check }: { check: VerificationStep }) {
  const passed = check.status === 'passed'
  return (
    <TraceLine status={passed ? undefined : 'error'}>
      <TraceIcon>{passed ? <CircleCheckIcon /> : <CircleXIcon />}</TraceIcon>
      <TraceTitle className="whitespace-normal">
        {passed ? 'Passed: ' : 'Not passed: '}
        {check.description}
        {check.summary ? ` ${check.summary}` : ''}
      </TraceTitle>
    </TraceLine>
  )
}
