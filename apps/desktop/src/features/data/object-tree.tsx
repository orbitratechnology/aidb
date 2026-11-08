import { useDeferredValue, useEffect, useRef, useState } from 'react'
import type { CatalogSnapshot, ObjectName, ObjectSummary } from '@aidb/contracts'
import { ChevronRightIcon, FolderIcon, LockIcon, SearchIcon } from 'lucide-react'
import { Link } from 'react-router'
import { Button } from '@/components/ui/button'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'
import { InputGroup, InputGroupAddon, InputGroupInput } from '@/components/ui/input-group'
import { cn } from '@/lib/utils'
import { pluralize } from '@/features/connections/format'
import {
  approximateCount,
  groupObjects,
  KIND_LABELS,
  objectPath,
  sameObject,
  type SchemaGroup,
} from './catalog-view'
import { KIND_ICONS } from './kind-display'

/** Objects rendered per schema before "Show more", so huge schemas stay responsive. */
const BATCH = 200

type TreeProps = {
  catalog: CatalogSnapshot
  selected: ObjectName | null
  /** Called after an object is picked, e.g. to close the sheet holding the tree. */
  onPick?: () => void
}

/** Searchable list of the catalog's tables and views, grouped by schema. */
export function ObjectTree({ catalog, selected, onPick }: TreeProps) {
  const [query, setQuery] = useState('')
  const deferredQuery = useDeferredValue(query)
  const groups = groupObjects(catalog, deferredQuery)
  const searching = deferredQuery.trim() !== ''
  const matches = groups.reduce((sum, group) => sum + group.objects.length, 0)

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex flex-col gap-2 border-b p-3">
        <InputGroup>
          <InputGroupAddon>
            <SearchIcon />
          </InputGroupAddon>
          <InputGroupInput
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search tables and views"
            aria-label="Search tables and views"
            autoComplete="off"
            spellCheck={false}
          />
        </InputGroup>
        <p className="text-xs text-muted-foreground" aria-live="polite">
          {searching
            ? pluralize(matches, 'match', 'matches')
            : `${pluralize(catalog.objects.length, 'object', 'objects')} in ${pluralize(catalog.schemas.length, 'schema', 'schemas')}`}
        </p>
      </div>
      <nav aria-label="Tables and views" className="min-h-0 flex-1 overflow-y-auto p-2">
        {groups.length === 0 ? (
          <p className="p-2 text-sm text-muted-foreground">
            {searching ? 'No tables or views match your search.' : 'This database has no schemas.'}
          </p>
        ) : (
          <ul className="flex flex-col gap-1">
            {groups.map((group) => (
              <SchemaSection
                // Remount on a new search so each result list starts at its first batch.
                key={`${group.schema.name}\u0000${deferredQuery}`}
                group={group}
                forceOpen={searching}
                defaultOpen={
                  catalog.schemas.length <= 3 ||
                  group.schema.name === 'public' ||
                  group.schema.name === selected?.schema
                }
                selected={selected}
                onPick={onPick}
              />
            ))}
          </ul>
        )}
      </nav>
    </div>
  )
}

function SchemaSection({
  group: { schema, objects },
  forceOpen,
  defaultOpen,
  selected,
  onPick,
}: {
  group: SchemaGroup
  forceOpen: boolean
  defaultOpen: boolean
  selected: ObjectName | null
  onPick?: () => void
}) {
  const [open, setOpen] = useState(defaultOpen)
  const [limit, setLimit] = useState(BATCH)

  // When the selection moves into this schema (a relationship link, back or forward), open it
  // and render far enough to include the selected object.
  const selectedName = selected?.schema === schema.name ? selected.name : null
  const [revealed, setRevealed] = useState<string | null>(null)
  if (selectedName !== revealed) {
    setRevealed(selectedName)
    if (selectedName !== null) {
      setOpen(true)
      const index = objects.findIndex((object) => object.name === selectedName)
      if (index >= limit) setLimit(Math.ceil((index + 1) / BATCH) * BATCH)
    }
  }
  const visible = objects.slice(0, limit)
  const remaining = objects.length - visible.length

  return (
    <li>
      <Collapsible open={forceOpen || open} onOpenChange={setOpen}>
        <CollapsibleTrigger asChild>
          <Button
            variant="ghost"
            size="sm"
            className="group/schema w-full justify-start gap-2 font-medium max-sm:min-h-11"
            disabled={forceOpen}
            title={schema.comment ?? undefined}
          >
            <ChevronRightIcon className="transition-transform group-data-[state=open]/schema:rotate-90" />
            <FolderIcon />
            <span className="truncate">{schema.name}</span>
            <span className="ml-auto text-xs font-normal text-muted-foreground">
              {schema.accessible ? objects.length : 'No access'}
            </span>
          </Button>
        </CollapsibleTrigger>
        <CollapsibleContent>
          {objects.length === 0 ? (
            <p className="py-1 pl-10 text-xs text-muted-foreground">
              {schema.accessible ? 'No tables or views' : 'You cannot list this schema'}
            </p>
          ) : (
            <ul className="flex flex-col py-1 pl-4">
              {visible.map((object) => (
                <ObjectRow
                  key={object.name}
                  object={object}
                  current={selected !== null && sameObject(object, selected)}
                  onPick={onPick}
                />
              ))}
              {remaining > 0 && (
                <li>
                  <Button
                    variant="link"
                    size="sm"
                    className="pl-6 max-sm:min-h-11"
                    onClick={() => setLimit((current) => current + BATCH)}
                  >
                    Show {Math.min(remaining, BATCH)} more of {remaining}
                  </Button>
                </li>
              )}
            </ul>
          )}
        </CollapsibleContent>
      </Collapsible>
    </li>
  )
}

function ObjectRow({
  object,
  current,
  onPick,
}: {
  object: ObjectSummary
  current: boolean
  onPick?: () => void
}) {
  const Icon = KIND_ICONS[object.kind]
  const rows = approximateCount(object.rowEstimate)
  const ref = useRef<HTMLAnchorElement>(null)

  useEffect(() => {
    if (current) ref.current?.scrollIntoView({ block: 'nearest' })
  }, [current])

  return (
    <li>
      <Link
        ref={ref}
        to={objectPath(object)}
        onClick={onPick}
        aria-current={current ? 'page' : undefined}
        title={object.comment ?? undefined}
        className={cn(
          'flex items-center gap-2 rounded-md px-2 py-1.5 text-sm outline-none hover:bg-accent hover:text-accent-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 max-sm:min-h-11 [&_svg]:size-4 [&_svg]:shrink-0',
          current && 'bg-accent font-medium text-accent-foreground',
        )}
      >
        <Icon aria-hidden className="text-muted-foreground" />
        <span className="sr-only">{KIND_LABELS[object.kind]}:</span>
        <span className="min-w-0 truncate">{object.name}</span>
        {!object.readable && (
          <>
            <LockIcon aria-hidden className="text-muted-foreground" />
            <span className="sr-only">(no read access)</span>
          </>
        )}
        {rows && (
          <span className="ml-auto shrink-0 text-xs text-muted-foreground tabular-nums">
            {rows}
            <span className="sr-only"> rows (estimate)</span>
          </span>
        )}
      </Link>
    </li>
  )
}
