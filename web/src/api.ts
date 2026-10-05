import { beginApiTrace, finishApiTrace } from './process';
let csrf = '';
export const setCsrf = (value: string) => {
  csrf = value;
};
export class ApiError extends Error {
  constructor(
    message: string,
    public status = 0,
    public code = 'NETWORK',
  ) {
    super(message);
  }
}
export async function api<T>(
  path: string,
  options: { method?: string; body?: unknown; key?: string; file?: File } = {},
): Promise<T> {
  const headers: Record<string, string> = { 'X-TruHabit-Request': 'web' };
  if (options.body !== undefined) headers['Content-Type'] = 'application/json';
  if (options.file) headers['Content-Type'] = 'application/octet-stream';
  if (csrf) headers['X-CSRF-Token'] = csrf;
  if (options.key) headers['Idempotency-Key'] = options.key;
  let res: Response;
  const trace = beginApiTrace(path, options.method ?? 'GET');
  try {
    res = await fetch(path, {
      method: options.method ?? 'GET',
      credentials: 'same-origin',
      headers,
      body: options.file ?? (options.body === undefined ? undefined : JSON.stringify(options.body)),
      signal: AbortSignal.timeout(options.file || path.startsWith('/api/prototype/') || path.includes('/programs/') ? 90000 : 12000),
    });
  } catch {
    finishApiTrace(trace, 0);
    throw new ApiError('Odpověď serveru nepřišla. Zkontrolujte spojení.');
  }
  if (!res.ok) {
    let text: string;
    try { text = await res.text(); }
    catch {
      finishApiTrace(trace, 0);
      throw new ApiError('Odpověď serveru nepřišla. Zkontrolujte spojení.', 0, 'NETWORK');
    }
    let message = 'Požadavek se nepodařilo dokončit.';
    let code = 'REQUEST_FAILED';
    try {
      const body = JSON.parse(text) as { error?: string; code?: string };
      message = body.error ?? message;
      code = body.code ?? code;
    } catch {
      if (res.status === 422) message = 'Zkontrolujte vyplněné údaje.';
    }
    finishApiTrace(trace, res.status, code, message);
    throw new ApiError(message, res.status, code);
  }
  try {
    const value = await res.json() as T;
    finishApiTrace(trace, res.status);
    return value;
  } catch {
    finishApiTrace(trace, res.status, 'INVALID_RESPONSE');
    throw new ApiError('Požadavek se nepodařilo dokončit.', res.status, 'INVALID_RESPONSE');
  }
}
