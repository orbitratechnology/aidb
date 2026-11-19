import { THEME_OPTIONS } from '@/components/shared/theme-options'
import { FieldDescription, FieldSet, FieldLegend } from '@/components/ui/field'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { useTheme } from '@/hooks/use-theme'
import { isTheme } from '@/lib/theme'
import { SettingsSection } from '../settings-section'

export function AppearanceSettings() {
  const { theme, setTheme } = useTheme()

  return (
    <SettingsSection title="Appearance" description="How aidb looks on this device.">
      <FieldSet>
        <FieldLegend variant="label">Theme</FieldLegend>
        <ToggleGroup
          type="single"
          variant="outline"
          spacing={0}
          value={theme}
          onValueChange={(value) => {
            // Radix reports an empty string when the active item is clicked again.
            if (isTheme(value)) setTheme(value)
          }}
          aria-label="Theme"
        >
          {THEME_OPTIONS.map(({ value, label, icon: Icon }) => (
            <ToggleGroupItem key={value} value={value} className="min-h-11 px-4 sm:min-h-0">
              <Icon />
              {label}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        <FieldDescription>System follows your operating system setting.</FieldDescription>
      </FieldSet>
    </SettingsSection>
  )
}
