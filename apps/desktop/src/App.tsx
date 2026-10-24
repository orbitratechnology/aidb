import { useEffect, useState } from 'react'
import { CircleAlertIcon } from 'lucide-react'
import { ThemeToggle } from '@/components/shared/theme-toggle'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'
import { getAppInfo, type AppInfo } from '@/lib/tauri'

type NativeStatus =
  | { state: 'loading' }
  | { state: 'ready'; info: AppInfo }
  | { state: 'error'; message: string }

// Temporary landing view; replaced by the app shell in task 1.3.
function App() {
  const [status, setStatus] = useState<NativeStatus>({ state: 'loading' })

  useEffect(() => {
    getAppInfo().then(
      (info) => setStatus({ state: 'ready', info }),
      (err: unknown) =>
        setStatus({ state: 'error', message: err instanceof Error ? err.message : String(err) }),
    )
  }, [])

  return (
    <main className="flex min-h-svh items-center justify-center p-4">
      <Card className="w-full max-w-md">
        <CardHeader>
          <CardTitle>aidb</CardTitle>
          <CardDescription>Local-first PostgreSQL client</CardDescription>
          <CardAction>
            <ThemeToggle />
          </CardAction>
        </CardHeader>
        <CardContent>
          {status.state === 'loading' && <Skeleton className="h-6 w-48" />}
          {status.state === 'ready' && (
            <Badge variant="outline" className="font-mono">
              Native core {status.info.version} · {status.info.platform}
            </Badge>
          )}
          {status.state === 'error' && (
            <Alert variant="destructive">
              <CircleAlertIcon />
              <AlertTitle>Native core unavailable</AlertTitle>
              <AlertDescription>{status.message}</AlertDescription>
            </Alert>
          )}
        </CardContent>
      </Card>
    </main>
  )
}

export default App
