import type { ConnectionDraft, ImportCandidate } from '@aidb/contracts'

export type ImportPlan = {
  /** Profiles to send to the core. */
  connections: ConnectionDraft[]
  /** Ready entries left out because they need unencrypted access the user has not allowed. */
  heldBack: number
  /** Entries the core will not import: duplicates and invalid entries. */
  unusable: number
  /** Whether any ready entry needs consent for unencrypted access. */
  needsUnencryptedConsent: boolean
}

/** Chooses what to import. The core applies the same rules again and reports anything skipped. */
export function planImport(
  candidates: readonly ImportCandidate[],
  allowUnencrypted: boolean,
): ImportPlan {
  const plan: ImportPlan = {
    connections: [],
    heldBack: 0,
    unusable: 0,
    needsUnencryptedConsent: false,
  }
  for (const candidate of candidates) {
    if (candidate.status !== 'ready' || !candidate.connection) {
      plan.unusable += 1
      continue
    }
    if (candidate.unencryptedRemote) {
      plan.needsUnencryptedConsent = true
      if (!allowUnencrypted) {
        plan.heldBack += 1
        continue
      }
    }
    plan.connections.push(candidate.connection)
  }
  return plan
}
