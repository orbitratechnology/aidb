import type { ReactNode } from 'react'

export function SettingsSection({
  title,
  description,
  children,
}: {
  title: string
  description: string
  children: ReactNode
}) {
  return (
    <section aria-labelledby="settings-section-title" className="flex flex-col gap-6">
      <header className="flex flex-col gap-1">
        <h2 id="settings-section-title" className="text-lg font-semibold">
          {title}
        </h2>
        <p className="text-sm text-muted-foreground">{description}</p>
      </header>
      {children}
    </section>
  )
}
