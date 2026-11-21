// A stand-in for the native core in end-to-end tests: answers each command from fixed,
// typed data. Responses still pass the app's own Zod validation, so a fixture that drifts
// from the contracts fails the tests.
import type {
  ActiveSession,
  AppInfo,
  AppSettings,
  CatalogSnapshot,
  ConnectionDetails,
  DiagnosticsInfo,
  HistoryEntry,
  ObjectDetails,
  ProviderDetails,
  QueryPlan,
  QueryRun,
  SavedQuery,
  SqlAnalysis,
  StorageSummary,
  TablePage,
} from '@aidb/contracts'

/** What state the app starts in. `large` is connected to a big database (performance tests). */
export type Scenario = 'connected' | 'disconnected' | 'large'

const CONNECTION = '6f1c2a40-0000-4000-8000-00000000c001'
const PROVIDER = '6f1c2a40-0000-4000-8000-00000000a001'
const NOW = '2026-11-20T10:00:00Z'

const connection: ConnectionDetails = {
  id: CONNECTION,
  engine: 'postgres',
  password: 'os_store',
  unencryptedRemote: false,
  createdAt: NOW,
  updatedAt: NOW,
  name: 'Shop production',
  host: 'db.internal.example',
  port: 5432,
  database: 'shop',
  username: 'analyst',
  sslMode: 'verify-full',
  caCertPath: null,
  connectTimeoutSecs: 10,
  environment: 'production',
  readOnly: true,
}

const session: ActiveSession = {
  connectionId: CONNECTION,
  capabilities: {
    engine: 'postgres',
    cancelStatements: true,
    statementTimeout: true,
    lockTimeout: true,
    readOnlySessions: true,
    transactionalDdl: true,
    tls: true,
  },
  server: {
    engine: 'postgres',
    version: '17.2',
    versionNumber: 170002,
    supportedVersion: true,
    user: 'analyst',
    database: 'shop',
    encrypted: true,
    tlsVersion: 'TLSv1.3',
    readOnly: true,
    inRecovery: false,
  },
  connectedAt: NOW,
}

const appSettings: AppSettings = {
  appearance: { theme: 'light' },
  safety: {
    maxRowsPerPage: 500,
    maxResultMegabytes: 16,
    statementTimeoutSecs: 30,
    lockTimeoutSecs: 5,
  },
  ai: { enabled: true, providerId: PROVIDER },
}

const provider: ProviderDetails = {
  id: PROVIDER,
  endpointUrl: 'http://127.0.0.1:11434/v1',
  key: 'not_saved',
  keyRequired: false,
  insecureRemote: false,
  features: { toolCalls: true, structuredOutput: 'json_schema' },
  checkedAt: NOW,
  createdAt: NOW,
  updatedAt: NOW,
  name: 'Local model',
  kind: 'ollama',
  endpoint: null,
  model: 'qwen2.5',
  requestTimeoutSecs: 120,
  contextBudgetTokens: null,
  allowInsecureHttp: false,
}

const catalog: CatalogSnapshot = {
  connectionId: CONNECTION,
  loadedAt: NOW,
  database: 'shop',
  fingerprint: 'f1',
  schemas: [
    { name: 'public', comment: null, accessible: true },
    { name: 'staging', comment: 'Copies for testing changes', accessible: true },
  ],
  objects: [
    {
      schema: 'public',
      name: 'customers',
      kind: 'table',
      rowEstimate: 12400,
      comment: 'People who ordered',
      readable: true,
    },
    {
      schema: 'public',
      name: 'order_items_with_a_rather_long_name',
      kind: 'table',
      rowEstimate: 1_204_330,
      comment: null,
      readable: true,
    },
    {
      schema: 'public',
      name: 'orders',
      kind: 'table',
      rowEstimate: 200000,
      comment: null,
      readable: true,
    },
    {
      schema: 'public',
      name: 'big_orders',
      kind: 'view',
      rowEstimate: null,
      comment: null,
      readable: true,
    },
    {
      schema: 'staging',
      name: 'orders',
      kind: 'table',
      rowEstimate: 50,
      comment: null,
      readable: true,
    },
  ],
  truncated: false,
}

const orders: ObjectDetails = {
  summary: {
    schema: 'public',
    name: 'orders',
    kind: 'table',
    rowEstimate: 200000,
    comment: null,
    readable: true,
  },
  columns: [
    {
      name: 'id',
      position: 1,
      dataType: 'integer',
      nullable: false,
      default: null,
      identity: null,
      generated: null,
      comment: null,
    },
    {
      name: 'customer_id',
      position: 2,
      dataType: 'integer',
      nullable: false,
      default: null,
      identity: null,
      generated: null,
      comment: 'Who placed it',
    },
    {
      name: 'total',
      position: 3,
      dataType: 'numeric(10,2)',
      nullable: false,
      default: null,
      identity: null,
      generated: null,
      comment: null,
    },
    {
      name: 'placed_at',
      position: 4,
      dataType: 'timestamp with time zone',
      nullable: false,
      default: 'now()',
      identity: null,
      generated: null,
      comment: null,
    },
    {
      name: 'note',
      position: 5,
      dataType: 'text',
      nullable: true,
      default: null,
      identity: null,
      generated: null,
      comment: null,
    },
  ],
  primaryKey: { name: 'orders_pkey', columns: ['id'] },
  uniqueConstraints: [],
  foreignKeys: [],
  referencedBy: [],
  indexes: [
    {
      name: 'orders_pkey',
      definition: 'CREATE UNIQUE INDEX orders_pkey ON public.orders USING btree (id)',
      unique: true,
      primary: true,
      method: 'btree',
    },
  ],
  definition: null,
}

function page(): TablePage {
  return {
    object: { schema: 'public', name: 'orders' },
    columns: [
      { name: 'id', dataType: 'integer', kind: 'number' },
      { name: 'customer_id', dataType: 'integer', kind: 'number' },
      { name: 'total', dataType: 'numeric(10,2)', kind: 'number' },
      { name: 'placed_at', dataType: 'timestamp with time zone', kind: 'date_time' },
      { name: 'note', dataType: 'text', kind: 'text' },
    ],
    omittedColumns: [],
    rows: Array.from({ length: 30 }, (_, i) => [
      String(i + 1),
      String((i * 7) % 50),
      (i * 3.25).toFixed(2),
      '2026-11-19 10:00:00+00',
      i % 4 === 0 ? null : `Gift wrap, deliver after ${i + 8}:00 to the side door please`,
    ]),
    truncatedCells: [],
    offset: 0,
    hasMore: true,
    byteLimitReached: false,
    order: [{ column: 'id', direction: 'asc' }],
    stableOrder: true,
    cellCharLimit: 2000,
  }
}

const history: HistoryEntry[] = [
  {
    id: 2,
    executedAt: NOW,
    connectionId: CONNECTION,
    sql: 'SELECT count(*) FROM orders',
    status: 'completed',
    rowCount: 1,
    elapsedMs: 12,
    errorCode: null,
  },
  {
    id: 1,
    executedAt: NOW,
    connectionId: CONNECTION,
    sql: 'DELETE FROM orders',
    status: 'refused',
    rowCount: null,
    elapsedMs: null,
    errorCode: null,
  },
]

const saved: SavedQuery[] = [
  {
    id: '6f1c2a40-0000-4000-8000-0000000000b1',
    name: 'Big orders this week',
    sql: 'SELECT * FROM orders WHERE total > 100',
    connectionId: CONNECTION,
    createdAt: NOW,
    updatedAt: NOW,
  },
]

function analysis(sql: string): SqlAnalysis {
  const end = { line: 1, column: sql.length + 1, offset: sql.length }
  return {
    statements: sql.trim()
      ? [
          {
            span: { start: { line: 1, column: 1, offset: 0 }, end },
            kind: 'read',
            reason: null,
            problemAt: null,
          },
        ]
      : [],
    problem: null,
  }
}

function result(sql: string): QueryRun {
  return {
    status: 'completed',
    statement: {
      start: { line: 1, column: 1, offset: 0 },
      end: { line: 1, column: sql.length + 1, offset: sql.length },
    },
    columns: [{ name: 'count', dataType: 'bigint', kind: 'number' }],
    rows: [['200000']],
    truncatedCells: [],
    offset: 0,
    hasMore: false,
    byteLimitReached: false,
    cellCharLimit: 2000,
    elapsedMs: 12,
  }
}

const plan: QueryPlan = {
  analyzed: false,
  planningMs: 0.1,
  executionMs: null,
  truncated: false,
  nodes: [
    {
      id: 0,
      parent: null,
      depth: 0,
      nodeType: 'Seq Scan',
      label: 'Seq Scan on public.orders',
      details: ['Filter: (orders.customer_id = 42)'],
      totalCost: 3340.6,
      planRows: 40,
      actualRows: null,
      loops: null,
      actualMs: null,
      selfMs: null,
    },
  ],
  suggestions: [
    {
      kind: 'missing_index',
      severity: 'warning',
      title: 'An index on public.orders (customer_id) may help',
      detail: 'The query reads the whole table to keep a few rows.',
      evidence: ['Seq Scan on public.orders is expected to read 200,000 rows and keep 40 (0.02%).'],
      node: 0,
      sql: 'CREATE INDEX CONCURRENTLY ON public.orders (customer_id);',
    },
  ],
}

const storage: StorageSummary = {
  connections: 1,
  savedQueries: 1,
  queryHistory: 2,
  agentTasks: 0,
  auditRecords: 4,
  schemaCacheEntries: 1,
}

const diagnostics: DiagnosticsInfo = {
  dataDir: '/home/me/.local/share/com.orbitratech.aidb',
  logDir: '/home/me/.local/share/com.orbitratech.aidb/logs',
  localStore: { state: 'ready', schemaVersion: 3, message: null },
  credentialStore: { backend: 'os_store', reason: null },
}

/** 10,000 tables and views over five schemas: half the catalog overview's cap. */
const largeCatalog: CatalogSnapshot = {
  ...catalog,
  schemas: ['public', 'sales', 'billing', 'ops', 'archive'].map((name) => ({
    name,
    comment: null,
    accessible: true,
  })),
  objects: Array.from({ length: 10_000 }, (_, i) => ({
    schema: ['public', 'sales', 'billing', 'ops', 'archive'][i % 5]!,
    name: `${i % 10 === 0 ? 'v' : 't'}${String(i).padStart(5, '0')}`,
    kind: i % 10 === 0 ? ('view' as const) : ('table' as const),
    rowEstimate: i * 37,
    comment: null,
    readable: true,
  })),
}

/** A full page: the Safety setting's 500 rows, twelve columns. */
function largePage(): TablePage {
  const base = page()
  const columns = Array.from({ length: 12 }, (_, i) => ({
    name: `column_${i}`,
    dataType: i % 3 === 0 ? 'integer' : 'text',
    kind: i % 3 === 0 ? ('number' as const) : ('text' as const),
  }))
  return {
    ...base,
    columns,
    rows: Array.from({ length: 500 }, (_, r) =>
      columns.map((_, c) => (c % 3 === 0 ? String(r * 12 + c) : `value ${r}-${c} with some text`)),
    ),
    order: [{ column: 'column_0', direction: 'asc' }],
  }
}

const appInfo: AppInfo = { name: 'aidb', version: '0.1.0', platform: 'linux' }

type Args = Record<string, unknown> | undefined

/** Answers a command, or rejects as the core would for one it does not know. */
export function handler(
  scenario: Scenario,
  theme: 'light' | 'dark',
): (command: string, args: Args) => unknown {
  const connected = scenario !== 'disconnected'
  const large = scenario === 'large'
  const settings: AppSettings = { ...appSettings, appearance: { theme } }
  const answers: Record<string, (args: Args) => unknown> = {
    app_info: () => appInfo,
    diagnostics: () => diagnostics,
    get_settings: () => settings,
    storage_summary: () => storage,
    list_connections: () => [connection],
    active_sessions: () => (connected ? [session] : []),
    list_providers: () => [provider],
    catalog_overview: () => (large ? largeCatalog : catalog),
    refresh_catalog: () => (large ? largeCatalog : catalog),
    describe_object: () => orders,
    read_table_page: () => (large ? largePage() : page()),
    analyze_sql: (args) => analysis(String(args?.sql ?? '')),
    run_query: (args) => result(String((args?.request as { sql?: string } | undefined)?.sql ?? '')),
    explain_query: () => plan,
    list_query_history: () => history,
    list_saved_queries: () => saved,
    list_pending_changes: () => [],
    list_audit_events: () => [],
    list_migration_records: () => [],
    cancel_operation: () => true,
  }
  return (command, args) => {
    const answer = answers[command]
    if (!answer) {
      ;(window as unknown as { __unhandled: string[] }).__unhandled.push(command)
      throw { code: 'unavailable', message: `No fixture for ${command}.`, retryable: false }
    }
    return answer(args)
  }
}
