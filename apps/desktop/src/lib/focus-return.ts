/**
 * Focus return for Radix dialogs and sheets opened from state rather than by their own Trigger.
 *
 * Radix gives focus back only to a dialog's Trigger, so closing one opened any other way left
 * keyboard focus on the page body. Spread these handlers on the dialog's content: opening records
 * what had focus, and closing returns there. Modal dialogs close in the reverse order they open,
 * so a stack covers nested ones.
 *
 * Don't give a field in such a dialog `autoFocus`: it takes focus before the dialog reports that
 * it opened, so the opener is lost. Radix already focuses the dialog's first field.
 */
const openers: (HTMLElement | null)[] = []

export const returnFocus = {
  onOpenAutoFocus: (event: Event) => {
    const active = document.activeElement
    const dialog = event.currentTarget as Node | null
    const opener =
      active instanceof HTMLElement && active !== document.body && !dialog?.contains(active)
        ? active
        : null
    openers.push(opener)
  },
  onCloseAutoFocus: (event: Event) => {
    const opener = openers.pop()
    if (opener?.isConnected) {
      event.preventDefault()
      opener.focus()
    }
  },
}
