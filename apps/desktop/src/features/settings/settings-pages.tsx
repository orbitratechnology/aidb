import { ChevronLeftIcon, ChevronRightIcon } from 'lucide-react'
import { Link, Navigate, NavLink, Outlet, useParams } from 'react-router'
import { PageContainer } from '@/components/shared/page'
import { Button } from '@/components/ui/button'
import {
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemMedia,
  ItemTitle,
} from '@/components/ui/item'
import { findSettingsSection, SETTINGS_SECTIONS } from './sections'

/**
 * `/settings` lists the sections. A selected section shows a side list on wider screens and a
 * back link on compact screens.
 */
export function SettingsLayout() {
  const { section } = useParams()

  if (!section) {
    return (
      <PageContainer className="max-w-2xl">
        <Outlet />
      </PageContainer>
    )
  }

  return (
    <PageContainer className="sm:flex-row sm:items-start">
      <nav aria-label="Settings" className="hidden w-48 shrink-0 sm:block">
        <ul className="flex flex-col gap-1">
          {SETTINGS_SECTIONS.map(({ id, title, icon: Icon }) => (
            <li key={id}>
              <Button
                variant={id === section ? 'secondary' : 'ghost'}
                asChild
                className="w-full justify-start"
              >
                <NavLink to={`/settings/${id}`}>
                  <Icon data-icon="inline-start" />
                  {title}
                </NavLink>
              </Button>
            </li>
          ))}
        </ul>
      </nav>
      <div className="flex min-w-0 flex-1 flex-col gap-4">
        <Button variant="ghost" size="sm" asChild className="min-h-11 self-start sm:hidden">
          <Link to="/settings">
            <ChevronLeftIcon data-icon="inline-start" />
            All settings
          </Link>
        </Button>
        <Outlet />
      </div>
    </PageContainer>
  )
}

export function SettingsOverview() {
  return (
    <nav aria-label="Settings">
      <ItemGroup className="gap-2">
        {SETTINGS_SECTIONS.map(({ id, title, summary, icon: Icon }) => (
          <Item key={id} variant="outline" asChild>
            <Link to={`/settings/${id}`}>
              <ItemMedia variant="icon">
                <Icon />
              </ItemMedia>
              <ItemContent>
                <ItemTitle>{title}</ItemTitle>
                <ItemDescription>{summary}</ItemDescription>
              </ItemContent>
              <ItemActions>
                <ChevronRightIcon aria-hidden className="size-4 text-muted-foreground" />
              </ItemActions>
            </Link>
          </Item>
        ))}
      </ItemGroup>
    </nav>
  )
}

export function SettingsSectionRoute() {
  const { section } = useParams()
  const match = findSettingsSection(section)

  if (!match) {
    return <Navigate to="/settings" replace />
  }

  const Section = match.component
  return <Section />
}
