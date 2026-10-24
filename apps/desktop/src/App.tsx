import { useEffect, useState } from 'react'
import { getAppInfo, type AppInfo } from './lib/tauri.ts'

function App() {
  const [info, setInfo] = useState<AppInfo | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    getAppInfo().then(setInfo, (err: unknown) => {
      setError(err instanceof Error ? err.message : String(err))
    })
  }, [])

  return (
    <main>
      <h1>aidb</h1>
      {info && (
        <p>
          Native core {info.version} on {info.platform}
        </p>
      )}
      {error && <p role="alert">Native core unavailable: {error}</p>}
    </main>
  )
}

export default App
