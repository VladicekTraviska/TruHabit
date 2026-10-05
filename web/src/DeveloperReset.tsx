import { useEffect, useId, useRef, useState, type FormEvent } from 'react';
import { ArrowClockwise, CaretDown, CheckCircle, Code, Database, LockKey, ShieldCheck, Trash, Users, WarningCircle } from '@phosphor-icons/react';
import { api, ApiError } from './api';
import { Field } from './components';
import { useLanguage } from './i18n';
import type { Session } from './types';
import './developer-reset.css';

type ResetCounts = {
  goals: number; personal_challenges: number; personal_activity_files: number; local_credit_movements: number;
  owned_workspaces: number; owned_programs: number; owned_workspace_memberships: number; owned_team_activity_files: number;
  foreign_memberships: number; foreign_enrollments: number; foreign_activity_files: number;
};
type ResetPreview = {
  enabled: true; allowed: boolean; fingerprint: string; counts: ResetCounts;
  owned_workspace_impacts: { id: string; name: string; member_count: number; program_count: number; activity_file_count: number }[];
  blockers: { code: string; count: number }[];
  preserved: { account: true; credentials: true; sessions: true; linked_wallet: true; blockchain: true; other_accounts: true; foreign_workspaces: true };
  network: 'LOCAL'; real_money: false;
};
type ResetResult = {
  ok: true; account_preserved: true; deleted: ResetCounts;
  remaining_local_balance: { available: number; locked: number; forfeited: number }; network: 'LOCAL'; real_money: false;
};
type ResetError = { code: string; phase: 'preview' | 'reset' | 'refresh' };

const countKeys: (keyof ResetCounts)[] = ['goals', 'personal_challenges', 'personal_activity_files', 'local_credit_movements', 'owned_workspaces', 'owned_programs', 'owned_workspace_memberships', 'owned_team_activity_files', 'foreign_memberships', 'foreign_enrollments', 'foreign_activity_files'];
const isCount = (value: unknown) => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
function isPreview(value: ResetPreview) {
  return value?.enabled === true && typeof value.allowed === 'boolean' && /^[a-f0-9]{64}$/i.test(value.fingerprint)
    && value.network === 'LOCAL' && value.real_money === false
    && !!value.counts && countKeys.every(key => isCount(value.counts[key]))
    && Array.isArray(value.blockers) && value.blockers.every(item => !!item && typeof item.code === 'string' && isCount(item.count))
    && Array.isArray(value.owned_workspace_impacts) && value.owned_workspace_impacts.every(item => !!item && typeof item.id === 'string' && typeof item.name === 'string' && isCount(item.member_count) && isCount(item.program_count) && isCount(item.activity_file_count))
    && !!value.preserved && ['account', 'credentials', 'sessions', 'linked_wallet', 'blockchain', 'other_accounts', 'foreign_workspaces'].every(key => value.preserved[key as keyof ResetPreview['preserved']] === true);
}
function isResetResult(value: ResetResult) {
  return value?.ok === true && value.account_preserved === true && value.network === 'LOCAL' && value.real_money === false
    && !!value.deleted && countKeys.every(key => isCount(value.deleted[key]))
    && !!value.remaining_local_balance && ['available', 'locked', 'forfeited'].every(key => value.remaining_local_balance[key as keyof ResetResult['remaining_local_balance']] === 0);
}
function errorCode(error: unknown) {
  if (!(error instanceof ApiError)) return 'REQUEST_FAILED';
  return /^[A-Z0-9_]{1,80}$/.test(error.message) ? error.message : error.code;
}

export function DeveloperReset({ session, disabled = false, onReset, onExpired, onBusyChange }: {
  session: Session; disabled?: boolean; onReset: () => void | Promise<void>; onExpired?: () => void; onBusyChange?: (busy: boolean) => void;
}) {
  const language = useLanguage();
  const p = (en: string, cs: string) => language === 'en' ? en : cs;
  const heading = useId(); const content = useId();
  const [expanded, setExpanded] = useState(false);
  const [unavailable, setUnavailable] = useState(false);
  const [operation, setOperation] = useState<'preview' | 'reset' | null>(null);
  const [preview, setPreview] = useState<ResetPreview | null>(null);
  const [confirmation, setConfirmation] = useState('');
  const [error, setError] = useState<ResetError | null>(null);
  const [completed, setCompleted] = useState(false);
  const [profileReloadFailed, setProfileReloadFailed] = useState(false);
  const mounted = useRef(true); const lock = useRef(false); const generation = useRef(0);
  const currentSession = useRef(session); currentSession.current = session;
  const busyCallback = useRef(onBusyChange); busyCallback.current = onBusyChange;
  const password = useRef<HTMLInputElement>(null);
  const message = useRef<HTMLDivElement>(null);
  const busy = operation !== null;
  const allowed = preview?.allowed === true && preview.blockers.length === 0;
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; generation.current++; busyCallback.current?.(false); }; }, []);
  useEffect(() => {
    generation.current++; setExpanded(false); setUnavailable(false); setPreview(null); setConfirmation(''); setError(null); setCompleted(false); setProfileReloadFailed(false);
    if (password.current) password.current.value = '';
  }, [session.user.id, session.csrf_token]);
  useEffect(() => { if (error || completed) message.current?.focus(); }, [error, completed]);
  const sameSession = (identity: string, token: string, revision: number) => mounted.current && currentSession.current.user.id === identity && currentSession.current.csrf_token === token && generation.current === revision;
  function clearConfirmation() { setConfirmation(''); if (password.current) password.current.value = ''; }
  async function fetchPreview() {
    const result = await api<ResetPreview>('/api/account/dev-reset');
    if (!isPreview(result)) throw new ApiError('INVALID_RESPONSE', 200, 'INVALID_RESPONSE');
    return result;
  }
  function recordError(cause: unknown, phase: ResetError['phase']) {
    const code = errorCode(cause);
    if (cause instanceof ApiError && cause.status === 404) { setUnavailable(true); setPreview(null); return; }
    setError({ code, phase });
    if (code === 'UNAUTHORIZED') onExpired?.();
  }
  async function loadPreview() {
    if (lock.current || disabled || !mounted.current) return;
    lock.current = true;
    const identity = session.user.id; const token = session.csrf_token; const revision = ++generation.current;
    setOperation('preview'); setError(null); setCompleted(false); setProfileReloadFailed(false); setPreview(null); clearConfirmation();
    try { const result = await fetchPreview(); if (sameSession(identity, token, revision)) setPreview(result); }
    catch (cause) { if (sameSession(identity, token, revision)) recordError(cause, 'preview'); }
    finally { lock.current = false; if (mounted.current) setOperation(null); }
  }
  function togglePreview() {
    if (busy || disabled) return;
    if (expanded) { setExpanded(false); setPreview(null); clearConfirmation(); }
    else { setExpanded(true); void loadPreview(); }
  }
  async function reset(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (lock.current || disabled || !preview || !allowed || confirmation !== 'RESET' || !password.current?.value) return;
    const fingerprint = preview.fingerprint;
    const enteredPassword = password.current.value;
    const identity = session.user.id; const token = session.csrf_token; const revision = ++generation.current;
    const afterReset = onReset;
    lock.current = true; setOperation('reset'); setError(null); setProfileReloadFailed(false); busyCallback.current?.(true);
    // The password is used only in this request; it is never logged or persisted in browser storage.
    if (password.current) password.current.value = '';
    try {
      const result = await api<ResetResult>('/api/account/dev-reset', { method: 'POST', body: { password: enteredPassword, confirmation: 'RESET', fingerprint } });
      if (!isResetResult(result)) throw new ApiError('INVALID_RESPONSE', 200, 'INVALID_RESPONSE');
      if (sameSession(identity, token, revision)) { setPreview(null); setConfirmation(''); setCompleted(true); }
      // The parent guards cleanup by the original user identity, including a completed request after navigation.
      try { await afterReset(); }
      catch { if (sameSession(identity, token, revision)) setProfileReloadFailed(true); }
    } catch (cause) {
      if (sameSession(identity, token, revision)) {
        const code = errorCode(cause); recordError(cause, 'reset'); clearConfirmation();
        if (code === 'DEV_RESET_PREVIEW_CHANGED' || code === 'DEV_RESET_BLOCKED') {
          setPreview(null);
          try { const fresh = await fetchPreview(); if (sameSession(identity, token, revision)) setPreview(fresh); }
          catch (refreshError) { if (sameSession(identity, token, revision) && refreshError instanceof ApiError && refreshError.status === 404) setUnavailable(true); }
        } else if (code === 'NETWORK' || code === 'INVALID_RESPONSE' || code === 'DEV_RESET_INVALID_FINGERPRINT') setPreview(null);
      }
    } finally {
      lock.current = false; busyCallback.current?.(false);
      if (mounted.current) setOperation(null);
    }
  }
  const countLabels: Record<keyof ResetCounts, string> = {
    goals: p('My plans', 'Moje plány'), personal_challenges: p('Personal challenges', 'Osobní výzvy'), personal_activity_files: p('Personal source files', 'Osobní zdrojové soubory'), local_credit_movements: p('LOCAL credit history', 'Historie LOCAL kreditů'),
    owned_workspaces: p('Workspaces I own', 'Prostory, které vlastním'), owned_programs: p('Programs in my workspaces', 'Programy v mých prostorech'), owned_workspace_memberships: p('Other members’ workspace memberships', 'Členství ostatních v mých prostorech'), owned_team_activity_files: p('Source files in my workspaces', 'Zdrojové soubory v mých prostorech'),
    foreign_memberships: p('My memberships elsewhere', 'Moje členství v jiných prostorech'), foreign_enrollments: p('My participation elsewhere', 'Moje účast v jiných prostorech'), foreign_activity_files: p('My source files elsewhere', 'Moje zdrojové soubory v jiných prostorech'),
  };
  const blockers: Record<string, { title: string; step: string; href?: string; link?: string }> = {
    DEV_RESET_DEVNET_ESCROW_ACTIVE: { title: p('An active Devnet stake is still locked', 'Aktivní Devnet vklad je stále uzamčený'), step: p('Settle the Devnet challenge using its agreed rules, then refresh this preview. Reset cannot unlock or erase blockchain funds.', 'Vypořádejte Devnet výzvu podle jejích sjednaných pravidel a potom obnovte náhled. Reset nemůže odemknout ani smazat prostředky na blockchainu.'), href: '/?view=prototype', link: p('Open Challenges', 'Otevřít Výzvy') },
    DEV_RESET_DEVNET_COMMAND_PENDING: { title: p('A Devnet transaction request is unfinished', 'Požadavek Devnet transakce není dokončený'), step: p('Open Challenges and inspect whether the request awaits a signature or chain confirmation. Complete or reconcile it under the challenge’s rules. If an unused request cannot be cleared, ask the installation administrator. Reset does not cancel or resubmit it.', 'Otevřete Výzvy a ověřte, zda požadavek čeká na podpis nebo potvrzení sítě. Dokončete nebo ověřte jej podle pravidel výzvy. Pokud nepoužitý požadavek nejde vyřešit, obraťte se na správce instalace. Reset jej neruší ani znovu neodesílá.'), href: '/?view=prototype', link: p('Open Challenges', 'Otevřít Výzvy') },
    DEV_RESET_DEVNET_STATE_UNVERIFIED: { title: p('Devnet state has not been verified', 'Stav Devnetu nebyl ověřen'), step: p('Refresh the challenge’s chain status. If the network is unavailable, retry this preview once the status can be checked.', 'Obnovte stav sítě u výzvy. Pokud je síť nedostupná, opakujte náhled, až bude možné stav ověřit.'), href: '/?view=prototype', link: p('Open Challenges', 'Otevřít Výzvy') },
    DEV_RESET_FOREIGN_PARTICIPATION_ACTIVE: { title: p('Participation in another workspace is active', 'Účast v jiném prostoru je aktivní'), step: p('Complete or settle that participation with the workspace owner. A program must respect its upload deadline, reviews and earned rewards before closing.', 'Dokončete nebo vypořádejte účast s vlastníkem prostoru. Před uzavřením musí program dodržet termín nahrání, posouzení i získané odměny.'), href: '/?view=business', link: p('Open team programs', 'Otevřít týmové programy') },
    DEV_RESET_FOREIGN_FINANCIAL_HISTORY: { title: p('Shared reward history must be retained', 'Sdílenou historii odměn je nutné zachovat'), step: p('This profile has financial records with another workspace. They cannot be removed by this reset. Keep these records or use a separate development account.', 'Profil má finanční záznamy s jiným prostorem. Tento reset je nemůže odstranit. Zachovejte tyto záznamy nebo použijte samostatný vývojový účet.') },
    DEV_RESET_SHARED_FINANCIAL_HISTORY: { title: p('Other members have linked reward history', 'Ostatní členové mají navázanou historii odměn'), step: p('This reset cannot remove records that affect another member’s credit history. Keep the shared history or use a separate development account.', 'Reset nemůže odstranit záznamy, které ovlivňují historii kreditů jiného člena. Zachovejte sdílenou historii nebo použijte samostatný vývojový účet.') },
  };
  const errorText = (item: ResetError) => {
    const messages: Record<string, string> = {
      DEV_RESET_CONFIRMATION_REQUIRED: p('Type RESET exactly to confirm.', 'Pro potvrzení napište přesně RESET.'),
      DEV_RESET_INVALID_FINGERPRINT: p('The preview could not be verified. Refresh it and review the counts again.', 'Náhled se nepodařilo ověřit. Obnovte jej a znovu projděte počty.'),
      DEV_RESET_PREVIEW_CHANGED: p('The data changed after your preview. Review the refreshed counts before confirming again.', 'Od načtení náhledu se data změnila. Před dalším potvrzením projděte obnovené počty.'),
      DEV_RESET_BLOCKED: p('The server blocked the reset. Resolve the blockers shown in the refreshed preview first.', 'Server reset zablokoval. Nejprve vyřešte překážky v obnoveném náhledu.'),
      INVALID_CREDENTIALS: p('The current password is incorrect. You remain signed in; enter it again.', 'Současné heslo není správné. Zůstáváte přihlášeni; zadejte je znovu.'),
      UNAUTHORIZED: p('Your session expired. Sign in again.', 'Platnost přihlášení vypršela. Přihlaste se znovu.'),
      FORBIDDEN: p('The server did not authorize this request. Refresh the page before retrying.', 'Server tento požadavek nepovolil. Před opakováním obnovte stránku.'),
      RATE_LIMITED: p('Too many attempts. Wait and try again later.', 'Příliš mnoho pokusů. Vyčkejte a zkuste to později.'),
      HASHING_BUSY: p('Password checking is busy. Try again in a moment.', 'Kontrola hesla je vytížená. Zkuste to za chvíli.'),
    };
    if (item.code === 'NETWORK' || item.code === 'INVALID_RESPONSE') return item.phase === 'reset'
      ? p('The reset response could not be verified. It may have completed. Refresh the preview before trying again.', 'Odpověď resetu se nepodařilo ověřit. Reset mohl být dokončen. Před opakováním obnovte náhled.')
      : p('The preview could not be loaded. Check the connection and refresh the preview.', 'Náhled se nepodařilo načíst. Zkontrolujte spojení a obnovte náhled.');
    return messages[item.code] ?? p('The operation could not be completed. Refresh the preview before retrying.', 'Operaci se nepodařilo dokončit. Před opakováním obnovte náhled.');
  };
  if (unavailable) return null;
  return <section className="developer-reset" aria-labelledby={heading} aria-busy={busy}>
    <div className="developer-reset-intro"><span className="developer-reset-icon"><Code size={25} aria-hidden="true"/></span><div><span className="developer-reset-label">{p('DEVELOPMENT / PROTOTYPE', 'VÝVOJ / PROTOTYP')}</span><h2 id={heading}>{p('Start again with a clean test profile.', 'Začněte znovu s čistým testovacím profilem.')}</h2><p>{p('Reset your prototype data while keeping this account and your sign-in. Review the exact scope first.', 'Vymažte prototypová data a zachovejte tento účet i přihlášení. Nejprve projděte přesný rozsah.')}</p></div></div>
    <button className="button secondary developer-reset-open" type="button" disabled={busy || disabled} aria-expanded={expanded} aria-controls={content} onClick={togglePreview}><Database size={18} aria-hidden="true"/>{expanded ? p('Close preview', 'Zavřít náhled') : p('Preview developer reset', 'Náhled vývojářského resetu')}<CaretDown size={16} aria-hidden="true"/></button>
    {expanded && <div className="developer-reset-content" id={content}>
      {operation === 'preview' && <p className="developer-reset-loading" role="status"><ArrowClockwise size={18} aria-hidden="true"/>{p('Loading the current reset scope…', 'Načítám aktuální rozsah resetu…')}</p>}
      {(error || completed) && <div ref={message} tabIndex={-1} className={'developer-reset-message' + (completed ? ' success' : ' error')} role={error ? 'alert' : 'status'}>{completed ? <CheckCircle size={22} aria-hidden="true"/> : <WarningCircle size={22} aria-hidden="true"/>}<div>{completed && <><strong>{p('Prototype reset completed.', 'Reset prototypu dokončen.')}</strong><p>{p('Your account, credentials, active sessions and linked wallet are preserved. Your existing blockchain records remain.', 'Váš účet, přihlašovací údaje, aktivní přihlášení i propojená peněženka zůstávají zachovány. Existující blockchainové záznamy zůstávají.')}</p></>}{error && <p>{errorText(error)}</p>}{profileReloadFailed && <p>{p('The reset succeeded, but the profile refresh failed. Reload the page to see the current profile.', 'Reset proběhl, ale obnovení profilu se nezdařilo. Pro aktuální profil obnovte stránku.')}</p>}</div></div>}
      {preview && <>
        <div className="developer-reset-preview-heading"><div><span className="developer-reset-step">{p('1 · REVIEW THE SCOPE', '1 · PROJDĚTE ROZSAH')}</span><h3>{p('Records included in this reset', 'Záznamy zahrnuté do resetu')}</h3><p>{p('These are the current counts from the server. The same scope is verified again before anything is removed.', 'Toto jsou aktuální počty ze serveru. Před smazáním se stejný rozsah znovu ověří.')}</p></div><button type="button" className="button text-button" disabled={busy || disabled} onClick={() => void loadPreview()}><ArrowClockwise size={15} aria-hidden="true"/>{p('Refresh preview', 'Obnovit náhled')}</button></div>
        <dl className="developer-reset-counts">{countKeys.map(key => <div key={key}><dt>{countLabels[key]}</dt><dd>{new Intl.NumberFormat(language === 'en' ? 'en-GB' : 'cs-CZ').format(preview.counts[key])}</dd></div>)}</dl>
        {preview.owned_workspace_impacts.length > 0 && <section className="developer-reset-workspaces"><h4><Users size={20} aria-hidden="true"/>{p('Your owned workspaces will be removed', 'Vaše vlastní prostory budou odstraněny')}</h4><p>{p('Their members will lose access to these workspaces. The listed programs, memberships and uploaded test files are removed too. Other members keep their own accounts and sign-in.', 'Jejich členové ztratí přístup k těmto prostorům. Odstraní se také uvedené programy, členství a nahrané testovací soubory. Ostatním členům zůstanou jejich účty i přihlášení.')}</p><ul>{preview.owned_workspace_impacts.map(workspace => <li key={workspace.id}><strong>{workspace.name}</strong><span>{p('Other members', 'Ostatní členové')}: {workspace.member_count} · {p('Programs', 'Programy')}: {workspace.program_count} · {p('Source files', 'Zdrojové soubory')}: {workspace.activity_file_count}</span></li>)}</ul></section>}
        <div className="developer-reset-preserved"><ShieldCheck size={22} aria-hidden="true"/><div><h4>{p('What stays', 'Co zůstane zachováno')}</h4><p>{p('Your account, profile identity, password, active sessions and linked wallet. Other people’s accounts and workspaces you do not own remain. Your participation in other workspaces is detached from your account; their program history remains.', 'Váš účet, totožnost profilu, heslo, aktivní přihlášení a propojená peněženka. Účty ostatních lidí a prostory, které nevlastníte, zůstanou zachovány. Vaše účast v jiných prostorech se odpojí od účtu; historie jejich programů zůstane.')}</p><p>{p('A database reset cannot erase blockchain history or change wallet balances. It does not send a blockchain transaction.', 'Reset databáze nemůže smazat historii blockchainu ani změnit zůstatky peněženky. Neodesílá blockchainovou transakci.')}</p></div></div>
        {!allowed && <section className="developer-reset-blockers" aria-label={p('Reset blockers', 'Překážky resetu')}><h4><LockKey size={20} aria-hidden="true"/>{p('Resolve these before resetting', 'Vyřešte před resetem')}</h4>{preview.blockers.length ? <ul>{preview.blockers.map(blocker => { const explanation = blockers[blocker.code]; return <li key={blocker.code}><div><strong>{explanation?.title ?? p('The server found protected records', 'Server nalezl chráněné záznamy')}</strong><span>{blocker.count}</span></div><p>{explanation?.step ?? p('Refresh the preview after the protected participation or financial records have been settled. This reset cannot bypass their rules.', 'Obnovte náhled po vypořádání chráněné účasti nebo finančních záznamů. Reset nemůže obejít jejich pravidla.')}</p>{explanation?.href && <a href={explanation.href}>{explanation.link}</a>}</li>; })}</ul> : <p>{p('The server has not authorized this reset. Refresh the preview to check its current eligibility.', 'Server tento reset nepovolil. Obnovte náhled a ověřte aktuální podmínky.')}</p>}</section>}
        <form className="developer-reset-confirm" onSubmit={event => void reset(event)}>
          <span className="developer-reset-step">{p('2 · CONFIRM THIS PREVIEW', '2 · POTVRĎTE TENTO NÁHLED')}</span><h3>{p('Reset the data shown above', 'Vymazat výše uvedená data')}</h3>
          <p>{p('This permanently clears your prototype records and source files from the active database and removes the owned workspaces shown above. It removes your memberships elsewhere and detaches your participation from those programs. Your LOCAL test-credit balance and history return to zero. Your account stays signed in.', 'Trvale vymaže vaše prototypové záznamy a zdrojové soubory z aktivní databáze a odstraní výše uvedené vlastní prostory. Zruší vaše členství jinde a odpojí vaši účast od těchto programů. Zůstatek i historie vašich LOCAL testovacích kreditů se vynulují. Účet zůstane přihlášený.')}</p>
          <input type="hidden" name="username" autoComplete="username" value={session.user.email}/>
          <fieldset disabled={busy || disabled || !allowed}>
            <div className="developer-reset-fields"><Field label={p('Current password', 'Současné heslo')}><input ref={password} name="password" type="password" autoComplete="current-password" required maxLength={512}/></Field><Field label={p('Type RESET exactly', 'Napište přesně RESET')} hint={p('Uppercase, without spaces.', 'Velkými písmeny, bez mezer.')}><input name="confirmation" value={confirmation} onChange={event => setConfirmation(event.target.value)} required pattern="RESET" minLength={5} maxLength={5} autoComplete="off" autoCapitalize="characters" spellCheck={false}/></Field></div>
            <button className="button developer-reset-submit" type="submit" disabled={busy || disabled || !allowed || confirmation !== 'RESET'}><Trash size={18} aria-hidden="true"/>{operation === 'reset' ? p('Resetting prototype data…', 'Mažu prototypová data…') : p('Reset my prototype data', 'Vymazat moje prototypová data')}</button>
          </fieldset>
          <p className="developer-reset-server-check"><ShieldCheck size={15} aria-hidden="true"/>{p('The server rechecks your password, this preview and all blockers before resetting.', 'Server před resetem znovu ověří heslo, tento náhled i všechny překážky.')}</p>
        </form>
      </>}
      {!preview && operation !== 'preview' && !completed && <button className="button secondary" type="button" disabled={busy || disabled} onClick={() => void loadPreview()}><ArrowClockwise size={17} aria-hidden="true"/>{p('Refresh preview', 'Obnovit náhled')}</button>}
    </div>}
  </section>;
}
