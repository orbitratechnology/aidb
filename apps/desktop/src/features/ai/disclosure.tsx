import { CircleSlashIcon, GlobeIcon, HouseIcon, SendIcon } from 'lucide-react'

/**
 * What the assistant sends to a provider, and what it never sends. The agent must keep to this;
 * it is the promise shown before AI is turned on.
 */
export function WhatIsSent({ destination }: { destination?: { local: boolean; host: string } }) {
  return (
    <div className="flex flex-col gap-4 text-sm">
      <div className="flex gap-3">
        <SendIcon aria-hidden className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
        <div className="flex flex-col gap-1">
          <p className="font-medium">Sent when you ask the assistant something</p>
          <ul className="list-disc ps-5 text-muted-foreground">
            <li>Your question and the conversation so far.</li>
            <li>
              The structure of the open connection: names and types of its schemas, tables, columns,
              keys and comments.
            </li>
            <li>
              Rows from read queries the assistant runs, and files you attach. A hosted provider
              gets them only after you allow it in that conversation; a provider on this computer
              gets them without asking, since nothing leaves it.
            </li>
          </ul>
        </div>
      </div>
      <div className="flex gap-3">
        <CircleSlashIcon aria-hidden className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
        <div className="flex flex-col gap-1">
          <p className="font-medium">Never sent</p>
          <ul className="list-disc ps-5 text-muted-foreground">
            <li>Database passwords, API keys or other credentials.</li>
            <li>Anything from connections other than the open one.</li>
          </ul>
        </div>
      </div>
      <div className="flex gap-3">
        {destination?.local === false ? (
          <GlobeIcon aria-hidden className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
        ) : (
          <HouseIcon aria-hidden className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
        )}
        <p className="text-muted-foreground">
          {destination === undefined
            ? 'A local provider such as Ollama keeps everything on this computer. A hosted provider receives it over the internet and handles it under its own data policies.'
            : destination.local
              ? `This provider runs on this computer (${destination.host}), so nothing leaves it.`
              : `This provider is hosted at ${destination.host}: what is sent goes there over the internet and is handled under its own data policies.`}
        </p>
      </div>
    </div>
  )
}
