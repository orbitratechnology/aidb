import { useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import type { ConnectionDetails } from '@aidb/contracts'
import { saveConnectionPassword } from '@/lib/commands'
import { NativeError } from '@/lib/native'
import { PasswordDialog, type PasswordPrompt } from './password-dialog'
import { CONNECTIONS_QUERY_KEY } from './queries'
import { useConnect } from './sessions'

/**
 * Connects to a profile. Without a saved password, or when the saved one is rejected, asks for
 * one first; it is saved only after it has worked, and only if the user asks.
 */
export function useConnectFlow() {
  const connect = useConnect()
  const queryClient = useQueryClient()
  const [prompt, setPrompt] = useState<PasswordPrompt | null>(null)

  const announce = (connection: ConnectionDetails, readOnly: boolean, encrypted: boolean) =>
    toast.success(`Connected to ${connection.name}`, {
      description: [
        readOnly ? 'Read-only' : 'Writes enabled',
        encrypted ? 'encrypted' : 'not encrypted',
      ].join(', '),
    })

  const connectTo = (connection: ConnectionDetails) => {
    if (connection.password === 'not_saved') {
      setPrompt({ connection, problem: null })
      return
    }
    connect.mutate(
      { id: connection.id },
      {
        onSuccess: (session) =>
          announce(connection, session.server.readOnly, session.server.encrypted),
        onError: (error) => {
          if (error instanceof NativeError && error.code === 'permission_denied') {
            setPrompt({ connection, problem: error.message })
          } else {
            toast.error(`Couldn't connect to ${connection.name}`, { description: error.message })
          }
        },
      },
    )
  }

  const submitPassword = (password: string, remember: boolean) => {
    if (!prompt) return
    const { connection } = prompt
    connect.mutate(
      { id: connection.id, password },
      {
        onSuccess: async (session) => {
          setPrompt(null)
          announce(connection, session.server.readOnly, session.server.encrypted)
          if (remember) {
            try {
              await saveConnectionPassword(connection.id, password)
            } catch (error) {
              toast.error("The password worked but couldn't be saved", {
                description: error instanceof Error ? error.message : undefined,
              })
            }
            await queryClient.invalidateQueries({ queryKey: CONNECTIONS_QUERY_KEY })
          }
        },
        onError: (error) => setPrompt({ connection, problem: error.message }),
      },
    )
  }

  const dialog = (
    <PasswordDialog
      prompt={prompt}
      pending={connect.isPending}
      onSubmit={submitPassword}
      onCancel={() => setPrompt(null)}
    />
  )

  return {
    connectTo,
    connectingId: connect.isPending ? connect.variables?.id : undefined,
    dialog,
  }
}
