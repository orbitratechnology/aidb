import type { ProviderDetails } from '@aidb/contracts'
import { MessageSquarePlusIcon, SettingsIcon, SparklesIcon } from 'lucide-react'
import { useEffect } from 'react'
import { Link } from 'react-router'
import { LoadError } from '@/components/shared/load-error'
import { PageContainer } from '@/components/shared/page'
import { EmptyState, RequireConnection } from '@/components/shared/status-states'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  MessageScroller,
  MessageScrollerButton,
  MessageScrollerContent,
  MessageScrollerItem,
  MessageScrollerProvider,
  MessageScrollerViewport,
} from '@/components/ui/message-scroller'
import { Skeleton } from '@/components/ui/skeleton'
import { isLocalProvider, providerBlocker } from '@/features/ai/provider-display'
import { useProviders } from '@/features/ai/queries'
import { useActiveConnection } from '@/features/connections/sessions'
import type { ActiveConnection } from '@/features/connections/types'
import { useSettings } from '@/hooks/use-settings'
import { useAsk } from './ask-store'
import { Composer } from './composer'
import { QuestionView, ReplyView } from './turn-view'

export function AskPage() {
  return (
    <RequireConnection
      icon={SparklesIcon}
      title="Ask about your data"
      description="Connect to a PostgreSQL database, then describe what you need in plain language. AI is optional and stays off until you set up a provider."
    >
      <AssistantGate />
    </RequireConnection>
  )
}

/** The workspace once the assistant is on and its provider is usable; otherwise what to do. */
function AssistantGate() {
  const active = useActiveConnection()
  const settings = useSettings()
  const providers = useProviders()

  if (settings.isPending || providers.isPending) {
    return (
      <PageContainer>
        <Skeleton className="h-40 w-full" />
      </PageContainer>
    )
  }
  if (settings.isError || providers.isError) {
    return (
      <PageContainer>
        <LoadError
          title="The assistant settings could not be loaded"
          error={settings.error ?? providers.error ?? new Error('Unknown error')}
          retrying={settings.isFetching || providers.isFetching}
          onRetry={() => {
            void settings.refetch()
            void providers.refetch()
          }}
        />
      </PageContainer>
    )
  }
  const ai = settings.data.ai
  const provider = providers.data.find((candidate) => candidate.id === ai.providerId)
  const blocker = provider ? providerBlocker(provider) : null
  if (!active || !ai.enabled || !provider || blocker) {
    return (
      <PageContainer>
        <EmptyState
          icon={SparklesIcon}
          title={blocker ? 'The assistant needs attention' : 'The assistant is off'}
          description={
            blocker ??
            'Choose an AI provider and turn the assistant on in Settings › AI. You stay in control of what is sent.'
          }
        >
          <Button asChild className="min-h-11 sm:min-h-0">
            <Link to="/settings/ai">
              <SettingsIcon data-icon="inline-start" />
              Open AI settings
            </Link>
          </Button>
        </EmptyState>
      </PageContainer>
    )
  }
  return <AskWorkspace connection={active} provider={provider} />
}

const SUGGESTIONS = [
  'What is in this database?',
  'Which tables are the largest?',
  'How are the main tables related?',
]

function AskWorkspace({
  connection,
  provider,
}: {
  connection: ActiveConnection
  provider: ProviderDetails
}) {
  const turns = useAsk((state) => state.turns)
  const conversationFor = useAsk((state) => state.connectionId)
  const running = useAsk((state) => state.runningTurn !== null)
  const reset = useAsk((state) => state.reset)
  const hosted = !isLocalProvider(provider)

  // A conversation belongs to one connection.
  useEffect(() => {
    if (conversationFor !== connection.id) reset(connection.id)
  }, [conversationFor, connection.id, reset])

  return (
    <div className="mx-auto flex min-h-0 w-full max-w-4xl flex-1 flex-col gap-3 p-4 sm:p-6">
      <header className="flex flex-wrap items-center gap-2">
        <h1 className="text-lg font-semibold">Ask</h1>
        <Badge variant="outline">
          {provider.name} · {provider.model}
        </Badge>
        <Badge variant="secondary">{hosted ? 'Hosted provider' : 'On this computer'}</Badge>
        <Badge variant="secondary">
          {connection.readOnly ? 'Reads only' : 'Proposes changes for your review'}
        </Badge>
        <Button
          variant="ghost"
          size="sm"
          className="ml-auto"
          disabled={turns.length === 0}
          onClick={() => reset(connection.id)}
        >
          <MessageSquarePlusIcon data-icon="inline-start" />
          New conversation
        </Button>
      </header>

      <MessageScrollerProvider autoScroll>
        <MessageScroller className="flex-1">
          <MessageScrollerViewport>
            <MessageScrollerContent className="gap-6 py-2">
              {turns.length === 0 ? (
                <MessageScrollerItem messageId="welcome">
                  <EmptyState
                    icon={SparklesIcon}
                    title={`Ask about ${connection.database}`}
                    description={
                      connection.readOnly
                        ? 'The assistant looks up the structure and runs read-only queries to answer. Every step it takes is listed with its result.'
                        : 'The assistant looks up the structure and runs read-only queries to answer. It can also prepare changes, which wait for you to review and apply. Every step it takes is listed with its result.'
                    }
                  >
                    <div className="flex flex-wrap justify-center gap-2">
                      {SUGGESTIONS.map((suggestion) => (
                        <Button
                          key={suggestion}
                          variant="outline"
                          size="sm"
                          disabled={running}
                          onClick={() => {
                            useAsk.getState().setQuestion(suggestion)
                            void useAsk.getState().send()
                          }}
                        >
                          {suggestion}
                        </Button>
                      ))}
                    </div>
                  </EmptyState>
                </MessageScrollerItem>
              ) : (
                turns.flatMap((turn) => [
                  <MessageScrollerItem
                    key={`${turn.id}-question`}
                    messageId={`${turn.id}-question`}
                    scrollAnchor
                  >
                    <QuestionView turn={turn} />
                  </MessageScrollerItem>,
                  <MessageScrollerItem key={`${turn.id}-reply`} messageId={`${turn.id}-reply`}>
                    <ReplyView turn={turn} providerName={provider.name} />
                  </MessageScrollerItem>,
                ])
              )}
            </MessageScrollerContent>
          </MessageScrollerViewport>
          <MessageScrollerButton />
        </MessageScroller>
      </MessageScrollerProvider>

      <Composer
        connectionId={connection.id}
        database={connection.database}
        provider={{ name: provider.name, hosted }}
      />
    </div>
  )
}
