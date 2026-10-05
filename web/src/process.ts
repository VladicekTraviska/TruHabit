import { useSyncExternalStore } from 'react';

export type ProcessEvent = {
  id: number; at: string; category: 'app' | 'network' | 'chain' | 'evidence';
  level: 'info' | 'success' | 'warning' | 'error'; en: string; cs: string; signature?: string;
};
let sequence = 0;
let epoch = 0;
let sessionScope = 0;
let entries: readonly ProcessEvent[] = [];
const listeners = new Set<() => void>();
const empty: readonly ProcessEvent[] = [];
// Deliberately ephemeral. Callers supply public observations, never request bodies,
// source files, authentication tokens, wallet signatures or serialized transactions.
export function emitProcess(event: Omit<ProcessEvent, 'id' | 'at'>, scope = sessionScope) {
  if (scope !== sessionScope) return;
  entries = [...entries.slice(-199), { ...event, en: event.en.slice(0, 350), cs: event.cs.slice(0, 350), id: ++sequence, at: new Date().toISOString() }];
  listeners.forEach(fn => fn());
}
export function clearProcess() { epoch++; entries = []; listeners.forEach(fn => fn()); }
export function resetProcessSession() { sessionScope++; clearProcess(); }
export function getProcessScope() { return sessionScope; }
export function getProcessSnapshot() { return entries; }
function subscribe(fn: () => void) { listeners.add(fn); return () => { listeners.delete(fn); }; }
export function useProcess() { return useSyncExternalStore(subscribe, getProcessSnapshot, () => empty); }

export function beginApiTrace(path: string, method: string) {
  const route = path.split('?')[0].replace(/[a-f0-9]{8}-[a-f0-9-]{27,}/gi, ':id');
  if (/(^|\/)source(?:\/|$)/.test(route)) return null;
  // Team activity and funding are LOCAL observations. Invitations, identity,
  // member emails, private-source routes and tokens are deliberately excluded.
  const teamProgram = /^\/api\/organizations\/:id\/programs(?:\/:id(?:\/(?:publish|join|close|archive|next-cycle)|\/enrollments\/:id(?:\/(?:upload|claim|review))?)?)?$/.test(route);
  const companyPoints = /^\/api\/organizations\/:id\/points(?:\/top-up)?$/.test(route);
  if (!path.startsWith('/api/prototype/') && !teamProgram && !companyPoints && route !== '/api/business/review') return null;
  const category = route.includes('/upload') ? 'evidence' : route === '/api/prototype/balance' || /\/(prepare|submit|refresh)$/.test(route) ? 'chain' : 'network';
  const started = performance.now();
  emitProcess({ category, level: 'info', en: `${method} ${route} · request started`, cs: `${method} ${route} · požadavek zahájen` });
  return { category, route, method, started, epoch } as const;
}
// Only public, explicitly known prerequisite identifiers may supplement a generic
// HTTP error code. Never copy arbitrary response text into the presentation log.
const publicReasons = new Set([
  'LINK_PHANTOM_FIRST', 'WRONG_NETWORK', 'ACTION_NOT_AVAILABLE', 'SETTLEMENT_PENDING',
  'INSUFFICIENT_TEST_TOKENS', 'INSUFFICIENT_TEST_SOL', 'INSUFFICIENT_SIMULATION_CREDITS',
  'INVALID_BUSINESS_TERMS', 'INVALID_BUSINESS_BUDGET', 'BUSINESS_VERSION_CHANGED',
  'BUSINESS_UPLOAD_WINDOW_OPEN', 'BUSINESS_ACCEPTED_REWARDS_MUST_BE_PAID', 'BUSINESS_REVIEW_PENDING',
  'CLOSE_BUSINESS_PROGRAM_FIRST', 'BUSINESS_HISTORY_MUST_BE_RETAINED', 'WORKSPACE_ARCHIVED',
  'INSUFFICIENT_COMPANY_POINTS', 'INSUFFICIENT_EMPLOYEE_POINTS', 'BUSINESS_CONSENT_REQUIRED',
  'BUSINESS_NEXT_CYCLE_EXISTS', 'BUSINESS_POINT_ID_CONFLICT',
]);
export function finishApiTrace(trace: ReturnType<typeof beginApiTrace>, status: number, code?: string, reason?: string) {
  if (!trace || trace.epoch !== epoch) return;
  const elapsed = Math.round(performance.now() - trace.started);
  const safeCode = code && /^[A-Z0-9_]{1,70}$/.test(code) ? ` · ${code}` : '';
  const safeReason = status >= 400 && reason !== code && typeof reason === 'string' && publicReasons.has(reason) ? ` · ${reason}` : '';
  emitProcess({ category: trace.category, level: status >= 200 && status < 300 && code !== 'INVALID_RESPONSE' ? 'success' : 'error',
    en: `${trace.method} ${trace.route} · ${status || 'connection interrupted'} · ${elapsed} ms${safeCode}${safeReason}`,
    cs: `${trace.method} ${trace.route} · ${status || 'spojení přerušeno'} · ${elapsed} ms${safeCode}${safeReason}` });
}
