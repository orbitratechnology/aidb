import { useEffect, useEffectEvent, useRef, useState } from 'react'
import type { ConnectionId, QueryPlan, Suggestion } from '@aidb/contracts'
import { useMutation } from '@tanstack/react-query'
import {
  CircleAlertIcon,
  CopyIcon,
  GaugeIcon,
  InfoIcon,
  ListTreeIcon,
  SparklesIcon,
  TriangleAlertIcon,
} from 'lucide-react'
import { useNavigate } from 'react-router'
import { toast } from 'sonner'
import { EmptyState } from '@/components/shared/status-states'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { CodeBlock, CodeBlockContent } from '@/components/ui/code-block'
import { Item, ItemContent, ItemDescription, ItemMedia, ItemTitle } from '@/components/ui/item'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { useAsk } from '@/features/ask/ask-store'
import { copyText } from '@/lib/clipboard'
import { cancelOperation, explainQuery } from '@/lib/commands'
import { errorMessage } from '@/lib/errors'
import {
  amount,
  assistantQuestion,
  duration,
  misestimate,
  planText,
  slowestNode,
  timeShare,
} from './plan-view'
import { returnFocus } from '@/lib/focus-return'

type PlanProps = {
  connectionId: ConnectionId
  /** The query to explain; the sheet is open while there is one. */
  sql: string | null
  onClose: () => void
}

/**
 * A query's plan as PostgreSQL chose it, and what aidb suggests from its numbers. Estimated on
 * opening; measured on request (the query runs, read-only, and is rolled back).
 */
export function PlanSheet({ connectionId, sql, onClose }: PlanProps) {
  return (
    <Sheet open={sql !== null} onOpenChange={(open) => !open && onClose()}>
      <SheetContent
        {...returnFocus}
        side="right"
        className="gap-0 data-[side=right]:w-full data-[side=right]:sm:max-w-2xl"
      >
        <SheetHeader className="border-b">
          <SheetTitle>Query plan</SheetTitle>
          <SheetDescription>
            How PostgreSQL runs this query, and what its numbers suggest. Suggestions are drawn from
            the plan and the tables' statistics; nothing here changes the database.
          </SheetDescription>
        </SheetHeader>
        {/* Mounted while open, so each query is explained afresh. */}
        {sql !== null && <PlanBody connectionId={connectionId} sql={sql} onClose={onClose} />}
      </SheetContent>
    </Sheet>
  )
}

function PlanBody({
  connectionId,
  sql,
  onClose,
}: {
  connectionId: ConnectionId
  sql: string
  onClose: () => void
}) {
  const navigate = useNavigate()
  const [highlight, setHighlight] = useState<number | null>(null)
  const operation = useRef<string | null>(null)
  const explain = useMutation({
    mutationFn: (analyze: boolean) => {
      const operationId = crypto.randomUUID()
      operation.current = operationId
      return explainQuery(connectionId, { sql, analyze, operationId })
    },
    onSettled: () => {
      operation.current = null
    },
  })
  const estimateOnOpen = useEffectEvent(() => explain.mutate(false))
  useEffect(() => {
    estimateOnOpen()
    return () => {
      // Closed while measuring: stop the query.
      if (operation.current) cancelOperation(connectionId, operation.current).catch(() => undefined)
    }
  }, [connectionId])

  const plan = explain.data
  const ask = () => {
    const state = useAsk.getState()
    if (state.connectionId !== connectionId) state.reset(connectionId)
    useAsk.getState().setQuestion(assistantQuestion(sql))
    onClose()
    void navigate('/ask')
  }

  return (
    <>
      <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-4">
        <CodeBlock size="sm">
          <CodeBlockContent wrap className="max-h-28 text-xs">
            {sql}
          </CodeBlockContent>
        </CodeBlock>

        <div className="flex flex-wrap items-center gap-2">
          {explain.isPending ? (
            <>
              <span className="flex items-center gap-2 text-sm text-muted-foreground" role="status">
                <Spinner />
                {explain.variables ? 'Running the query to measure it…' : 'Planning…'}
              </span>
              {explain.variables && (
                <Button
                  variant="link"
                  size="sm"
                  className="h-auto px-0"
                  onClick={() =>
                    operation.current &&
                    cancelOperation(connectionId, operation.current).catch(() => undefined)
                  }
                >
                  Cancel
                </Button>
              )}
            </>
          ) : (
            plan && (
              <>
                <Badge variant="outline">{plan.analyzed ? 'Measured' : 'Estimated'}</Badge>
                {plan?.executionMs !== null && plan?.executionMs !== undefined && (
                  <span className="text-sm text-muted-foreground">
                    Ran in {duration(plan.executionMs)}
                    {plan.planningMs !== null && `, planned in ${duration(plan.planningMs)}`}.
                  </span>
                )}
                {!plan.analyzed && (
                  <Button
                    variant="outline"
                    size="sm"
                    className="max-sm:min-h-11"
                    onClick={() => explain.mutate(true)}
                  >
                    <GaugeIcon data-icon="inline-start" />
                    Run to measure
                  </Button>
                )}
              </>
            )
          )}
        </div>
        {plan && !plan.analyzed && !explain.isPending && (
          <p className="text-xs text-muted-foreground">
            Measuring runs the query in a read-only transaction that is rolled back, within the
            connection's statement timeout.
          </p>
        )}

        {explain.isError && (
          <Alert variant="destructive">
            <CircleAlertIcon />
            <AlertTitle>The query could not be explained</AlertTitle>
            <AlertDescription>{errorMessage(explain.error)}</AlertDescription>
          </Alert>
        )}
        {explain.isPending && !plan && <Skeleton className="h-40 w-full" />}

        {plan && (
          <>
            <section aria-labelledby="plan-suggestions" className="flex flex-col gap-2">
              <h3 id="plan-suggestions" className="text-sm font-medium">
                Suggestions
              </h3>
              {plan.suggestions.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  Nothing in this plan stands out.
                  {!plan.analyzed && ' Measuring it may show more.'}
                </p>
              ) : (
                plan.suggestions.map((suggestion, index) => (
                  <SuggestionItem
                    key={`${suggestion.kind}:${index}`}
                    suggestion={suggestion}
                    onShowNode={setHighlight}
                  />
                ))
              )}
            </section>
            <PlanTree plan={plan} highlight={highlight} />
          </>
        )}
      </div>
      <SheetFooter className="flex-row flex-wrap border-t">
        <Button
          variant="outline"
          className="max-sm:min-h-11"
          disabled={!plan}
          onClick={() =>
            plan &&
            void copyText(planText(plan)).then(
              () => toast.success('Plan copied.'),
              (error: unknown) => toast.error(errorMessage(error)),
            )
          }
        >
          <CopyIcon data-icon="inline-start" />
          Copy plan
        </Button>
        <Button className="max-sm:min-h-11" onClick={ask}>
          <SparklesIcon data-icon="inline-start" />
          Ask the assistant
        </Button>
      </SheetFooter>
    </>
  )
}

function SuggestionItem({
  suggestion,
  onShowNode,
}: {
  suggestion: Suggestion
  onShowNode: (node: number) => void
}) {
  const warning = suggestion.severity === 'warning'
  return (
    <Item variant="outline" size="sm" className="items-start">
      <ItemMedia variant="icon">
        {warning ? <TriangleAlertIcon className="text-destructive" /> : <InfoIcon />}
      </ItemMedia>
      <ItemContent className="min-w-0 gap-2">
        <ItemTitle className="flex flex-wrap items-center gap-2">
          <span className="break-words">{suggestion.title}</span>
          <Badge variant={warning ? 'destructive' : 'secondary'}>
            {warning ? 'Worth a look' : 'Note'}
          </Badge>
        </ItemTitle>
        <ItemDescription className="line-clamp-none">{suggestion.detail}</ItemDescription>
        <div className="flex flex-col gap-1">
          <span className="text-xs font-medium">Evidence</span>
          <ul className="list-disc pl-5 text-xs break-words text-muted-foreground">
            {suggestion.evidence.map((line) => (
              <li key={line}>{line}</li>
            ))}
          </ul>
        </div>
        {suggestion.sql && (
          <div className="flex flex-col gap-1">
            <CodeBlock size="sm">
              <CodeBlockContent wrap className="text-xs">
                {suggestion.sql}
              </CodeBlockContent>
            </CodeBlock>
            <span className="text-xs text-muted-foreground">
              Not run by aidb: review it and run it where you manage the schema.
            </span>
          </div>
        )}
        <div className="flex flex-wrap gap-2">
          {suggestion.sql && (
            <Button
              variant="outline"
              size="sm"
              className="max-sm:min-h-11"
              onClick={() =>
                void copyText(suggestion.sql ?? '').then(
                  () => toast.success('Copied.'),
                  (error: unknown) => toast.error(errorMessage(error)),
                )
              }
            >
              <CopyIcon data-icon="inline-start" />
              Copy SQL
            </Button>
          )}
          {suggestion.node !== null && (
            <Button
              variant="ghost"
              size="sm"
              className="max-sm:min-h-11"
              onClick={() => suggestion.node !== null && onShowNode(suggestion.node)}
            >
              <ListTreeIcon data-icon="inline-start" />
              Show in the plan
            </Button>
          )}
        </div>
      </ItemContent>
    </Item>
  )
}

function PlanTree({ plan, highlight }: { plan: QueryPlan; highlight: number | null }) {
  const slowest = slowestNode(plan)
  const highlighted = useRef<HTMLLIElement | null>(null)
  useEffect(() => {
    highlighted.current?.scrollIntoView({ behavior: 'smooth', block: 'center' })
  }, [highlight])
  if (plan.nodes.length === 0) {
    return (
      <EmptyState
        icon={ListTreeIcon}
        title="No plan"
        description="PostgreSQL returned an empty plan."
      />
    )
  }
  return (
    <section aria-labelledby="plan-tree" className="flex flex-col gap-2">
      <h3 id="plan-tree" className="text-sm font-medium">
        Plan
      </h3>
      <ol className="flex flex-col gap-1">
        {plan.nodes.map((node) => {
          const off = misestimate(node)
          const share = timeShare(node, plan)
          return (
            <li
              key={node.id}
              ref={node.id === highlight ? highlighted : undefined}
              aria-current={node.id === highlight ? 'true' : undefined}
              // Depth by indentation, capped so deep plans stay readable on narrow screens.
              style={{ marginLeft: `${Math.min(node.depth, 8) * 0.75}rem` }}
              className="rounded-md border p-2 text-sm aria-[current=true]:border-ring aria-[current=true]:ring-2 aria-[current=true]:ring-ring/40"
            >
              <div className="flex flex-wrap items-center gap-2">
                <span className="font-medium break-words">{node.label}</span>
                {node.id === slowest && <Badge variant="destructive">Slowest step</Badge>}
                {off && <Badge variant="outline">Estimate {off}</Badge>}
              </div>
              <div className="flex flex-wrap gap-x-3 text-xs text-muted-foreground tabular-nums">
                <span>cost {node.totalCost.toFixed(2)}</span>
                <span>
                  rows {amount(node.planRows)}
                  {node.actualRows !== null && ` est., ${amount(node.actualRows)} actual`}
                </span>
                {node.loops !== null && node.loops > 1 && <span>{amount(node.loops)} loops</span>}
                {node.selfMs !== null && (
                  <span>
                    {duration(node.selfMs)} here{share !== null && ` (${share}%)`}
                  </span>
                )}
              </div>
              {node.details.length > 0 && (
                <ul className="mt-1 flex flex-col font-mono text-xs break-all text-muted-foreground">
                  {node.details.map((detail) => (
                    <li key={detail}>{detail}</li>
                  ))}
                </ul>
              )}
            </li>
          )
        })}
      </ol>
      {plan.truncated && (
        <p className="text-xs text-muted-foreground">
          The plan is long; later steps are not shown.
        </p>
      )}
    </section>
  )
}
