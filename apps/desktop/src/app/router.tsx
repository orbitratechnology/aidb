import { createHashRouter, Navigate } from 'react-router'
import { AppShell } from './app-shell'

// Hash history works from the bundled `tauri://` origin without any server-side fallback.
// Feature screens load lazily so heavy editors and grids never weigh on startup.
export const router = createHashRouter([
  {
    path: '/',
    element: <AppShell />,
    children: [
      { index: true, element: <Navigate to="/ask" replace /> },
      {
        path: 'ask',
        lazy: async () => ({ Component: (await import('@/features/ask/ask-page')).AskPage }),
      },
      {
        path: 'data',
        lazy: async () => ({ Component: (await import('@/features/data/data-page')).DataPage }),
      },
      {
        path: 'sql',
        lazy: async () => ({ Component: (await import('@/features/sql/sql-page')).SqlPage }),
      },
      {
        path: 'changes',
        lazy: async () => ({
          Component: (await import('@/features/changes/changes-page')).ChangesPage,
        }),
      },
      {
        path: 'settings',
        lazy: async () => ({
          Component: (await import('@/features/settings/settings-pages')).SettingsLayout,
        }),
        children: [
          {
            index: true,
            lazy: async () => ({
              Component: (await import('@/features/settings/settings-pages')).SettingsOverview,
            }),
          },
          {
            path: ':section',
            lazy: async () => ({
              Component: (await import('@/features/settings/settings-pages')).SettingsSectionRoute,
            }),
          },
        ],
      },
      { path: '*', element: <Navigate to="/ask" replace /> },
    ],
  },
])
