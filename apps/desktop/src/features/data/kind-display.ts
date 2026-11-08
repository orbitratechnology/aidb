import {
  EyeIcon,
  GlobeIcon,
  LayersIcon,
  ScanEyeIcon,
  Table2Icon,
  type LucideIcon,
} from 'lucide-react'
import type { ObjectKind } from '@aidb/contracts'

// Each kind has an icon and a text label; the label is always available to assistive technology.
export const KIND_ICONS: Record<ObjectKind, LucideIcon> = {
  table: Table2Icon,
  partitioned_table: LayersIcon,
  view: EyeIcon,
  materialized_view: ScanEyeIcon,
  foreign_table: GlobeIcon,
}
