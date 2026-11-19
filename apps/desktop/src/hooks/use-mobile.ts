import { useMediaQuery } from '@/hooks/use-media-query'

// Matches the compact breakpoint (Tailwind `sm`); below it the app uses bottom navigation.
const MOBILE_BREAKPOINT = 640

export function useIsMobile(): boolean {
  return useMediaQuery(`(max-width: ${MOBILE_BREAKPOINT - 1}px)`)
}
