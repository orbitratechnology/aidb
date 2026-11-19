import { NavLink } from 'react-router'
import { PRIMARY_NAV } from './navigation'

/** Compact-screen navigation. Labels stay visible; active items get a shape cue, not just color. */
export function BottomNav() {
  return (
    <nav
      aria-label="Primary"
      className="shrink-0 border-t bg-background pb-[env(safe-area-inset-bottom)] sm:hidden"
    >
      <ul className="grid grid-cols-4">
        {PRIMARY_NAV.map(({ to, label, icon: Icon }) => (
          <li key={to}>
            <NavLink
              to={to}
              className="group/nav flex min-h-14 flex-col items-center justify-center gap-1 text-xs font-medium text-muted-foreground outline-none focus-visible:ring-3 focus-visible:ring-ring/50 aria-[current=page]:text-foreground"
            >
              <span className="flex h-7 w-12 items-center justify-center rounded-full transition-colors group-aria-[current=page]/nav:bg-accent">
                <Icon aria-hidden className="size-5" />
              </span>
              {label}
            </NavLink>
          </li>
        ))}
      </ul>
    </nav>
  )
}
