import { useEffect, useState } from 'react'

type VersionInfo = { version: string; protocol: number }

type ServerState =
  | { status: 'loading' }
  | { status: 'ok'; info: VersionInfo }
  | { status: 'error'; message: string }

export default function App() {
  const [server, setServer] = useState<ServerState>({ status: 'loading' })

  useEffect(() => {
    fetch('/api/v0/version')
      .then(async (res) => {
        if (!res.ok) throw new Error(`HTTP ${res.status}`)
        setServer({ status: 'ok', info: (await res.json()) as VersionInfo })
      })
      .catch((err: unknown) => setServer({ status: 'error', message: String(err) }))
  }, [])

  return (
    <main style={{ padding: 24 }}>
      <h1>losscape</h1>
      {server.status === 'loading' && <p style={{ color: 'var(--muted)' }}>Connecting…</p>}
      {server.status === 'ok' && (
        <p style={{ color: 'var(--ok)' }}>
          Server v{server.info.version}, protocol {server.info.protocol}
        </p>
      )}
      {server.status === 'error' && (
        <p style={{ color: 'var(--err)' }}>Server unreachable: {server.message}</p>
      )}
    </main>
  )
}
