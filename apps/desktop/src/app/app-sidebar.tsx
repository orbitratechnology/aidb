import { Link, NavLink, useLocation } from 'react-router'
import { AppMark } from '@/components/shared/app-mark'
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarRail,
} from '@/components/ui/sidebar'
import { isActivePath, PRIMARY_NAV, SETTINGS_NAV, type NavItem } from './navigation'

function NavMenuItem({ item, pathname }: { item: NavItem; pathname: string }) {
  const Icon = item.icon
  return (
    <SidebarMenuItem>
      <SidebarMenuButton asChild isActive={isActivePath(pathname, item.to)} tooltip={item.label}>
        <NavLink to={item.to}>
          <Icon />
          <span>{item.label}</span>
        </NavLink>
      </SidebarMenuButton>
    </SidebarMenuItem>
  )
}

/** Desktop and medium-width navigation. Collapses to icons; hidden on compact screens. */
export function AppSidebar() {
  const { pathname } = useLocation()

  return (
    <Sidebar collapsible="icon">
      <SidebarHeader>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton size="lg" asChild tooltip="aidb">
              <Link to="/ask">
                <AppMark />
                <span className="flex flex-col leading-tight">
                  <span className="font-semibold">aidb</span>
                  <span className="text-xs text-muted-foreground">PostgreSQL client</span>
                </span>
              </Link>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarHeader>
      <SidebarContent>
        <SidebarGroup>
          <SidebarGroupLabel>Workspace</SidebarGroupLabel>
          <SidebarGroupContent>
            <nav aria-label="Primary">
              <SidebarMenu>
                {PRIMARY_NAV.map((item) => (
                  <NavMenuItem key={item.to} item={item} pathname={pathname} />
                ))}
              </SidebarMenu>
            </nav>
          </SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>
      <SidebarFooter>
        <SidebarMenu>
          <NavMenuItem item={SETTINGS_NAV} pathname={pathname} />
        </SidebarMenu>
      </SidebarFooter>
      <SidebarRail />
    </Sidebar>
  )
}
