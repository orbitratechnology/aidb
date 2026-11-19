import { CircleAlertIcon, RotateCwIcon } from 'lucide-react'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'

export function LoadError({
  title,
  error,
  retrying,
  onRetry,
}: {
  title: string
  error: Error
  retrying: boolean
  onRetry: () => void
}) {
  return (
    <Alert variant="destructive">
      <CircleAlertIcon />
      <AlertTitle>{title}</AlertTitle>
      <AlertDescription className="flex flex-col items-start gap-3">
        <p>{error.message}</p>
        <Button
          variant="outline"
          size="sm"
          className="min-h-11 sm:min-h-0"
          disabled={retrying}
          onClick={onRetry}
        >
          {retrying ? (
            <Spinner data-icon="inline-start" />
          ) : (
            <RotateCwIcon data-icon="inline-start" />
          )}
          Try again
        </Button>
      </AlertDescription>
    </Alert>
  )
}
