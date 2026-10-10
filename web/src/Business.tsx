import { useEffect, useRef, useState, type FormEvent } from 'react';
import { Archive, ArrowRight, Buildings, CalendarBlank, CaretDown, ClockCounterClockwise, Handshake, Plus, ShieldCheck, Target, Trophy, Users, Wallet } from '@phosphor-icons/react';
import { api, ApiError } from './api';
import { Field, Message } from './components';
import { date, money, type Session } from './types';
import { useLanguage } from './i18n';
import { BusinessProgram } from './BusinessProgram';
import { BusinessWorkspaceLifecycle } from './BusinessWorkspaceLifecycle';
import { TeamMembers } from './TeamMembers';
import { ProgramDraftForm } from './ProgramDraftForm';
import { BusinessPoints } from './BusinessPoints';
import { companyPath, credits, points, templateName, type CompanyProgram, type Organization, type OrganizationDetail, type ProgramInput } from './business-types';
import './sections-ui.css';
import './business-ui.css';

type ReviewItem = { id: string; organization_id: string; program_id: string; program_title: string; display_name: string };
export function Business({ session, onExpired }: { session: Session; onExpired: () => void }) {
  const language = useLanguage(); const p = (en: string, cs: string) => language === 'en' ? en : cs;
  const [organizations, setOrganizations] = useState<Organization[]>([]);
  const [archivedOrganizations, setArchivedOrganizations] = useState<Organization[]>([]);
  const [detail, setDetail] = useState<OrganizationDetail | null>(null);
  const [loading, setLoading] = useState(true); const [selecting, setSelecting] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [busy, setBusy] = useState(false); const [error, setError] = useState(''); const [notice, setNotice] = useState<{ en: string; cs: string } | null>(null);
  const [newCompany, setNewCompany] = useState(false); const [editing, setEditing] = useState<CompanyProgram | 'new' | null>(null);
  const [program, setProgram] = useState<CompanyProgram | null>(null); const [tab, setTab] = useState<'programs' | 'members'>('programs');
  const [companyId, setCompanyId] = useState(() => crypto.randomUUID());
  const [accepting, setAccepting] = useState(false); const [invitationToken, setInvitationToken] = useState('');
  const [operator, setOperator] = useState(false); const [reviewQueue, setReviewQueue] = useState<ReviewItem[]>([]);
  const [operatorEnrollment, setOperatorEnrollment] = useState<string | undefined>();
  const [pointsRevision, setPointsRevision] = useState(0);
  const lock = useRef(false); const selection = useRef(0); const active = useRef(true);
  const errorPanel = useRef<HTMLDivElement>(null);
  useEffect(() => { if (error) errorPanel.current?.focus(); }, [error]);
  const org = detail?.organization; const manage = org?.role === 'OWNER' || org?.role === 'ADMIN';
  const archived = !!org?.archived_at;
  const activePrograms = detail?.programs.filter(item => item.state !== 'ARCHIVED') ?? [];
  const archivedPrograms = detail?.programs.filter(item => item.state === 'ARCHIVED') ?? [];

  function failed(e: unknown) {
    if (e instanceof ApiError && e.code === 'UNAUTHORIZED') { onExpired(); return; }
    const code = e instanceof ApiError && /^[A-Z0-9_]+$/.test(e.message) ? e.message : e instanceof ApiError ? e.code : '';
    const errors: Record<string, string> = {
      INSUFFICIENT_SIMULATION_CREDITS: p('Not enough LOCAL test credits. Get credits in Challenges, or reduce the program size.', 'Nemáte dost LOCAL testovacích kreditů. Získejte kredity ve Výzvách nebo snižte velikost programu.'),
      INVALID_BUSINESS_TERMS: p('Check the reward and dates. A new-run program must start at least five minutes in the future.', 'Zkontrolujte odměnu a termíny. Program pro nový běh musí začínat alespoň za pět minut.'),
      INVALID_BUSINESS_POINT_TERMS: p('Set a reward from 1 to 100,000 company points. Employer Match also requires a deposit from 1 to 100,000 points; other templates use no deposit. Reward × places must not exceed 10,000,000 points.', 'Odměna musí být od 1 do 100 000 firemních bodů. Spoluúčast vyžaduje také vklad od 1 do 100 000 bodů; ostatní šablony jsou bez vkladu. Odměna × počet míst nesmí překročit 10 000 000 bodů.'),
      INSUFFICIENT_COMPANY_POINTS: p('The company pool is too small. The owner can add demo points to the pool above, then try again.', 'Firemní pool nestačí. Vlastník může výše doplnit demo body do poolu a zkusit to znovu.'),
      INSUFFICIENT_EMPLOYEE_POINTS: p('You need previously earned company points for this voluntary deposit. Join a points-for-activity program first.', 'Pro tento dobrovolný vklad potřebujete už získané firemní body. Nejprve se zapojte do programu Body za aktivitu.'),
      BUSINESS_NEXT_CYCLE_EXISTS: p('The next monthly draft already exists. Open it from the program list.', 'Návrh dalšího měsíce už existuje. Otevřete jej ze seznamu programů.'),
      EMPLOYEE_POINTS_MUST_REMAIN_ACCESSIBLE: p('This member has earned or locked points. Keep their membership so they retain access.', 'Člen má získané nebo uzamčené body. Zachovejte jeho členství, aby k nim měl přístup.'),
      BUSINESS_CONSENT_REQUIRED: p('Read and explicitly accept this program’s deposit or bonus rules before joining.', 'Před účastí si přečtěte pravidla vkladu nebo bonusu a výslovně s nimi souhlaste.'),
      BUSINESS_REWARD_NOT_AVAILABLE: p('The reward is not available yet. Refresh the participation status.', 'Odměna ještě není dostupná. Obnovte stav účasti.'),
      BUSINESS_ACCEPTED_REWARDS_MUST_BE_PAID: p('Accepted rewards must be claimed before closure.', 'Přijaté odměny je nutné vyzvednout před uzavřením.'),
      BUSINESS_REVIEW_PENDING: p('Complete the pending operator review or wait until the review deadline.', 'Dokončete čekající operátorské posouzení nebo počkejte do termínu posouzení.'),
      BUSINESS_PROGRAM_FULL: p('The program has no free places. Your balance was not charged.', 'Program nemá volná místa. Váš zůstatek se neodečetl.'),
      INVITATION_INVALID: p('The invitation is expired, revoked or addressed to another email. Ask for a new code.', 'Pozvánka je prošlá, odvolaná nebo určená jinému e-mailu. Požádejte o nový kód.'),
      ACTIVITY_ALREADY_USED: p('This activity already counted toward another personal or team challenge. Submit a different run.', 'Tato aktivita už splnila jinou osobní nebo týmovou výzvu. Nahrajte jiný běh.'),
      UPLOAD_BODY_TIMEOUT: p('The complete file did not arrive within 30 seconds. Retry on a stable connection.', 'Celý soubor nebyl přijat do 30 sekund. Zkuste to znovu se stabilním připojením.'),
      INVALID_UPLOAD_BODY: p('Choose a valid GPX/FIT file up to 16 MiB.', 'Vyberte platný GPX/FIT do 16 MiB.'),
    };
    errors.BUSINESS_UPLOAD_WINDOW_OPEN = p('The upload window is still open. Close after the deadline or when all available places have been rewarded.', 'Období nahrávání ještě běží. Uzavřete po termínu nebo po vyplacení všech dostupných míst.');
    errors.BUSINESS_JOIN_CLOSED = p('Enrollment has closed. Refresh the program.', 'Přihlašování skončilo. Obnovte program.');
    errors.BUSINESS_UPLOAD_CLOSED = p('The upload deadline has passed or participation is already settled.', 'Termín nahrání uplynul nebo je účast již vypořádaná.');
    errors.BUSINESS_REVIEW_CLOSED = p('The review deadline has passed or participation is already settled.', 'Termín posouzení uplynul nebo je účast již vypořádaná.');
    errors.BUSINESS_VERSION_CHANGED = p('This record changed. Refresh before trying again.', 'Záznam se změnil. Před opakováním obnovte údaje.');
    errors.INVALID_BUSINESS_BUDGET = p('Reduce the reward or number of places: the budget limit is 10,000,000 company points, or 1,000 LOCAL credits for legacy programs.', 'Snižte odměnu nebo počet míst: limit rozpočtu je 10 000 000 firemních bodů, u původních programů 1 000 LOCAL kreditů.');
    errors.ACTIVE_BUSINESS_ENROLLMENT = p('Settle this member’s active participation before removal.', 'Před odebráním vypořádejte aktivní účast člena.');
    errors.BUSINESS_HISTORY_MUST_BE_RETAINED = p('This workspace has funding history. Close its programs, then use Archive workspace to remove it from the active list.', 'Prostor má historii financování. Uzavřete jeho programy a tlačítkem Archivovat prostor jej odeberte z aktivního seznamu.');
    errors.WORKSPACE_HAS_OTHER_MEMBERS = p('This workspace has other members. Use archiving after its programs are closed.', 'Prostor má další členy. Po uzavření programů použijte archivaci.');
    errors.WORKSPACE_HAS_PUBLISHED_PROGRAMS = p('Close the published programs first. Their details explain the remaining deadlines and rewards.', 'Nejprve uzavřete zveřejněné programy. Jejich detaily vysvětlují zbývající lhůty a odměny.');
    errors.WORKSPACE_RESERVED_CREDITS = p('Settle the reserved program credits before archiving.', 'Před archivací vypořádejte rezervované kredity programů.');
    errors.WORKSPACE_ARCHIVED = p('This workspace is archived. Its owner can restore it before making changes.', 'Prostor je archivovaný. Vlastník jej může před dalšími změnami obnovit.');
    errors.BUSINESS_OWNERSHIP_LIMIT = p('You already own ten active workspaces. Archive one before restoring another.', 'Už vlastníte deset aktivních prostorů. Před obnovením dalšího jeden archivujte.');
    errors.CLOSE_BUSINESS_PROGRAM_FIRST = p('Close the funded program before archiving.', 'Před archivací uzavřete financovaný program.');
    errors.FORBIDDEN = p('Your account does not have permission for this action.', 'Váš účet nemá oprávnění k této akci.');
    errors.NOT_FOUND = p('The record or invitation is unavailable for this account. Check the invitation email or refresh.', 'Záznam nebo pozvánka nejsou tomuto účtu dostupné. Zkontrolujte e-mail pozvánky nebo obnovte údaje.');
    errors.SELECT_FIT_SESSION = p('This FIT contains multiple sessions. Choose its activity number, starting at 1, then upload again.', 'FIT obsahuje více aktivit. Zvolte číslo aktivity od 1 a nahrajte znovu.');
    errors.FIT_SESSION_REQUIRED = p('This FIT has no usable activity session. Export the original running activity from your device app.', 'FIT neobsahuje použitelnou aktivitu. Exportujte původní běžeckou aktivitu z aplikace zařízení.');
    errors.INVALID_FIT_SESSION = p('This FIT activity number is not available. Choose an existing activity, starting at 1.', 'Toto číslo aktivity ve FIT není dostupné. Zvolte existující aktivitu, číslovanou od 1.');
    errors.SESSION_NOT_APPLICABLE = p('Activity selection is only for FIT files. Select the GPX file again to clear the activity number.', 'Výběr aktivity je určený pouze pro FIT. Vyberte soubor GPX znovu pro vymazání čísla aktivity.');
    errors.RUNNING_SESSION_REQUIRED = p('The selected FIT activity is not recorded as running. Choose the running activity or export a run.', 'Vybraná aktivita FIT není označená jako běh. Vyberte běžeckou aktivitu nebo exportujte běh.');
    errors.RATE_LIMITED = errors.UPLOAD_LIMIT_REACHED = p('The submission limit has been reached. Try later or start another program.', 'Limit nahrávání byl dosažen. Zkuste to později nebo vytvořte jiný program.');
    errors.PARSER_BUSY = p('Activity checking is busy. Retry in a moment.', 'Kontrola aktivit je vytížená. Zkuste to za chvíli.');
    errors.EXPECTED_GPX_OR_FIT = errors.EXPECTED_GPX_1_1 = p('Choose an original GPX 1.1 or FIT file.', 'Vyberte původní soubor GPX 1.1 nebo FIT.');
    for (const parseCode of ['INVALID_FIT', 'INVALID_FIT_LENGTH', 'INVALID_FIT_CRC_OR_FORMAT', 'INVALID_GPX']) errors[parseCode] = p('The file is incomplete or invalid. Export the original recording again.', 'Soubor je neúplný nebo neplatný. Exportujte původní záznam znovu.');
    errors.ACTIVITY_IN_FUTURE = p('The recording is dated in the future. Check its device clock.', 'Záznam má datum v budoucnosti. Zkontrolujte čas zařízení.');
    errors.REVIEW_REASON_REQUIRED = p('Provide a reason between 5 and 500 characters.', 'Vyplňte důvod v délce 5 až 500 znaků.');
    errors.CANNOT_OVERRIDE_GOAL_PARAMETERS = p('This run misses the agreed distance or dates. A review cannot change those rules.', 'Běh nesplňuje sjednanou vzdálenost nebo termín. Posouzení nemůže tato pravidla změnit.');
    setError(errors[code] ?? (e instanceof Error ? e.message : p('The operation failed. Refresh before retrying.', 'Operace se nezdařila. Před opakováním obnovte stav.')));
  }
  async function list() {
    const result = await api<{ organizations: Organization[]; archived_organizations?: Organization[]; is_operator?: boolean }>('/api/organizations');
    if (!active.current) return;
    setOrganizations(result.organizations); setArchivedOrganizations(result.archived_organizations ?? []); setOperator(result.is_operator ?? false);
    if (result.is_operator) { const reviews = await api<{ enrollments: ReviewItem[] }>('/api/business/review'); if (active.current) setReviewQueue(reviews.enrollments); }
  }
  async function select(id: string) {
    const revision = ++selection.current; setSelectedId(id); setSelecting(true); setDetail(null); setEditing(null); setProgram(null); setOperatorEnrollment(undefined); setTab('programs'); setError(''); setNotice(null);
    try { const result = await api<OrganizationDetail>(companyPath(id)); if (active.current && selection.current === revision) { setDetail(result); setPointsRevision(value => value + 1); } }
    catch (e) { if (active.current && selection.current === revision) failed(e); }
    finally { if (active.current && selection.current === revision) setSelecting(false); }
  }
  async function refresh(id: string) {
    const revision = selection.current; const result = await api<OrganizationDetail>(companyPath(id));
    if (!active.current || selection.current !== revision) return;
    setDetail(result); setSelectedId(id); setSelecting(false); setPointsRevision(value => value + 1); await list();
  }
  useEffect(() => { active.current = true; void list().catch(failed).finally(() => { if (active.current) setLoading(false); }); return () => { active.current = false; selection.current++; }; }, []);
  async function run(action: () => Promise<void>) {
    if (lock.current) return; lock.current = true; setBusy(true); setError(''); setNotice(null);
    try { await action(); } catch (e) { if (active.current) failed(e); }
    finally { lock.current = false; if (active.current) setBusy(false); }
  }
  function createCompany(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); const name = String(new FormData(event.currentTarget).get('company_name'));
    void run(async () => { const result = await api<Organization>('/api/organizations', { method: 'POST', body: { id: companyId, name } }); await select(result.id); await list(); setCompanyId(crypto.randomUUID()); setNewCompany(false); setNotice({ en: 'Workspace created. Add a program and invite your team.', cs: 'Prostor vytvořen. Přidejte program a pozvěte tým.' }); });
  }
  async function saveProgram(input: ProgramInput) {
    if (!org) return;
    await run(async () => { if (editing && editing !== 'new') { const { id, ...fields } = input; await api(companyPath(org.id, id), { method: 'PATCH', body: { ...fields, version: editing.version } }); } else await api(`${companyPath(org.id)}/programs`, { method: 'POST', body: input }); await refresh(org.id); setEditing(null); setNotice({ en: 'Draft saved. Open it to fund and publish.', cs: 'Návrh uložen. Otevřete jej pro financování a zveřejnění.' }); });
  }
  function acceptInvitation(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); const token = invitationToken.trim();
    void run(async () => { const result = await api<{ organization_id: string }>('/api/organization-invitations/accept', { method: 'POST', body: { token } }); setInvitationToken(''); setAccepting(false); await list(); await select(result.organization_id); setNotice({ en: 'Invitation accepted. Choose a published program to join.', cs: 'Pozvánka přijata. Vyberte zveřejněný program k účasti.' }); });
  }
  async function workspaceChanged(deleted: boolean) {
    if (!org) return;
    if (deleted) { selection.current++; setDetail(null); setSelectedId(null); setEditing(null); setProgram(null); await list(); setNotice({ en: 'Workspace deleted.', cs: 'Prostor smazán.' }); }
    else { await refresh(org.id); setEditing(null); setProgram(null); setNotice(archived ? { en: 'Workspace restored to the active list.', cs: 'Prostor obnoven do aktivního seznamu.' } : { en: 'Workspace archived. Find it under Archived workspaces.', cs: 'Prostor archivován. Najdete jej v Archivovaných prostorech.' }); }
  }
  function programCard(item: CompanyProgram) {
    const pointProgram = !!item.template && item.template !== 'LEGACY';
    const Icon = item.template === 'EVENT' ? Trophy : item.template === 'EMPLOYER_MATCH' ? Handshake : item.template === 'MONTHLY_BUDGET' ? CalendarBlank : Target;
    return <article className={`company-program company-program-row template-${(item.template ?? 'LEGACY').toLowerCase()}`} key={item.id}>
      <div className="company-program-identity"><span className="company-program-symbol"><Icon size={26} weight="duotone" aria-hidden="true" /></span><div><p className="company-program-type">{templateName(item.template, language)}</p><h3>{item.title}</h3>{item.template === 'EMPLOYER_MATCH' && <p className="field-hint">{p('Voluntary deposit:', 'Dobrovolný vklad:')} {points(item.point_stake, language)} {p('own earned points', 'vlastních získaných bodů')}</p>}</div></div>
      <dl className="company-program-facts"><div><dt>{p('Run', 'Běh')}</dt><dd>{item.target_m / 1000} <small>km</small></dd></div><div><dt>{p('Capacity', 'Kapacita')}</dt><dd>{item.max_participants} <small>{p('places', 'míst')}</small></dd></div><div><dt>{item.template === 'MONTHLY_BUDGET' ? p('Maximum bonus', 'Maximální bonus') : item.published_at || pointProgram ? p('Reward / person', 'Odměna / osoba') : p('Estimate / person', 'Odhad / osoba')}</dt><dd>{pointProgram ? points(item.point_reward, language) : item.published_at ? credits(item.reward_units, language) : money(item.reward_minor, item.currency)}{(pointProgram || item.published_at) && <small> {pointProgram ? p('points', 'bodů') : p('credits', 'kreditů')}</small>}</dd></div></dl>
      <div className="company-program-controls"><span className={`badge company-state state-${item.state.toLowerCase()}`}><span aria-hidden="true" />{item.state === 'DRAFT' ? p('Draft', 'Návrh') : item.state === 'PUBLISHED' ? p('Published', 'Zveřejněno') : item.state === 'CLOSED' ? p('Closed', 'Uzavřeno') : p('Archived', 'Archivováno')}</span><div className="detail-actions"><button className="button secondary" disabled={busy} onClick={() => { setError(''); setProgram(item); setOperatorEnrollment(undefined); }} aria-label={`${p('Open program', 'Otevřít program')}: ${item.title}`}>{p('Open', 'Otevřít')}<ArrowRight size={17} aria-hidden="true" /></button>{manage && !archived && item.state === 'DRAFT' && <button className="button text-button" disabled={busy} onClick={() => setEditing(item)}>{p('Edit draft', 'Upravit návrh')}</button>}</div></div>
    </article>;
  }
  function workspaceOption(item: Organization) {
    return <button key={item.id} className={`workspace-option ${item.id === selectedId ? 'selected' : ''}`} aria-pressed={item.id === selectedId} disabled={busy} onClick={() => void select(item.id)}><span className="workspace-symbol" aria-hidden="true">{item.archived_at ? <Archive size={20} aria-hidden="true" /> : item.name.slice(0, 2).toUpperCase()}</span><span><strong>{item.name}</strong><small>{item.role === 'OWNER' ? p('Owner', 'Vlastník') : item.role === 'ADMIN' ? p('Administrator', 'Správce') : p('Member', 'Člen')}</small></span><ArrowRight size={16} aria-hidden="true" /></button>;
  }
  async function openReview(item: ReviewItem) {
    const result = await api<{ organization: Organization; program: CompanyProgram }>(companyPath(item.organization_id, item.program_id));
    selection.current++;
    setDetail({ organization: result.organization, programs: [result.program], events: [] });
    setSelectedId(item.organization_id); setProgram(result.program); setOperatorEnrollment(item.id); setTab('programs');
  }
  return <div className="business-page sections-page" aria-busy={busy}>
    <section className="product-heading"><div><p className="eyebrow">{p('TRUHABIT FOR TEAMS', 'TRUHABIT PRO TÝMY')}</p><h1>{p('Movement that earns benefits.', 'Pohyb, který přináší benefity.')}</h1><p className="lead">{p('A company pool. Four voluntary programs. Points for a qualifying result.', 'Firemní pool. Čtyři dobrovolné programy. Body za vyhovující výsledek.')}</p></div><div className="company-row-actions"><button className="button secondary" disabled={busy} onClick={() => setAccepting(value => !value)}>{p('Accept invitation', 'Přijmout pozvánku')}</button><button className="button primary" disabled={busy} onClick={() => { setNewCompany(true); setEditing(null); }}><Plus size={18} aria-hidden="true" />{p('New workspace', 'Nový prostor')}</button></div></section>
    <div className="company-intro-flow" aria-label={p('How team programs work', 'Jak fungují týmové programy')}><div><span><Wallet size={21} aria-hidden="true" /></span><p><strong>{p('Company-funded rewards', 'Odměny hrazené firmou')}</strong>{p('A point pool funds each program.', 'Bodový pool financuje každý program.')}</p></div><div><span><Users size={21} aria-hidden="true" /></span><p><strong>{p('Voluntary participation', 'Dobrovolná účast')}</strong>{p('Your team chooses to take part.', 'Členové týmu si sami zvolí účast.')}</p></div><div><span><ShieldCheck size={21} aria-hidden="true" /></span><p><strong>{p('Private activity checks', 'Soukromá kontrola aktivity')}</strong>{p('HR sees results, not your recording.', 'HR vidí výsledek, nikoli váš záznam.')}</p></div></div>
    <p className="company-prototype-context"><span className="company-context-dot" aria-hidden="true" />{p('Functional prototype · simulated benefit points · no wallet connection needed', 'Funkční prototyp · simulované benefitní body · bez připojení peněženky')}</p>
    {error && <div className="business-error-focus" tabIndex={-1} ref={errorPanel}><Message error><span>{error}</span><button className="button secondary" disabled={busy} onClick={() => void run(async () => { if (org && !operatorEnrollment) await refresh(org.id); else await list(); })}>{p('Refresh data', 'Obnovit údaje')}</button></Message></div>}{notice && <Message><span>{p(notice.en, notice.cs)}</span></Message>}
    {accepting && <form className="panel settings-card company-create" onSubmit={acceptInvitation}><h2>{p('Accept your team invitation', 'Přijměte pozvánku do týmu')}</h2><fieldset disabled={busy}><Field label={p('Private invitation code', 'Soukromý kód pozvánky')} hint={p('Sign in with the email the invitation was addressed to.', 'Přihlaste se e-mailem, pro který je pozvánka určená.')}><textarea required maxLength={200} rows={2} value={invitationToken} onChange={event => setInvitationToken(event.target.value)} autoComplete="off" spellCheck={false} /></Field><div className="form-actions"><button className="button primary">{p('Accept invitation', 'Přijmout pozvánku')}</button><button className="button text-button" type="button" onClick={() => { setAccepting(false); setInvitationToken(''); }}>{p('Cancel', 'Zrušit')}</button></div></fieldset></form>}
    {newCompany && <form className="panel settings-card company-create" onSubmit={createCompany}><h2>{p('Create a team workspace', 'Vytvořit týmový prostor')}</h2><fieldset disabled={busy}><Field label={p('Company or team name', 'Název firmy nebo týmu')}><input name="company_name" minLength={2} maxLength={100} required autoFocus /></Field><p className="field-hint">{p('A private workspace does not verify a company or create a paid subscription.', 'Soukromý prostor neověřuje firmu ani nezakládá placené předplatné.')}</p><div className="form-actions"><button className="button primary">{p('Save workspace', 'Uložit prostor')}</button><button className="button text-button" type="button" onClick={() => setNewCompany(false)}>{p('Cancel', 'Zrušit')}</button></div></fieldset></form>}
    {operator && <details className="panel settings-card"><summary>{p('Operator review queue', 'Fronta operátorského posouzení')} · {reviewQueue.length}</summary>{reviewQueue.length ? reviewQueue.map(item => <div key={item.id} className="company-member-row"><span><strong>{item.program_title}</strong><small>{item.display_name}</small></span><button className="button secondary" disabled={busy} onClick={() => void run(() => openReview(item))}>{p('Review evidence', 'Posoudit podklad')}</button></div>) : <p className="field-hint">{p('No team evidence needs review.', 'Žádný týmový podklad nečeká na posouzení.')}</p>}</details>}
    <div className="business-layout"><aside className="panel settings-card workspace-rail"><div className="workspace-rail-title"><h2>{p('Your workspaces', 'Vaše prostory')}</h2><span>{organizations.length}</span></div>{loading ? <p role="status">{p('Loading…', 'Načítám…')}</p> : !organizations.length ? <p className="body-copy">{p('Create a workspace or accept an invitation. Personal challenges stay private.', 'Vytvořte prostor nebo přijměte pozvánku. Osobní výzvy zůstávají soukromé.')}</p> : <div className="workspace-options">{organizations.map(workspaceOption)}</div>}{!!archivedOrganizations.length && <details className="section-disclosure workspace-archive-list" key={archived ? 'selected-archive' : 'active-list'} open={archived || undefined}><summary>{p('Archived workspaces', 'Archivované prostory')} · {archivedOrganizations.length}</summary><div className="workspace-options">{archivedOrganizations.map(workspaceOption)}</div></details>}</aside>
      <section className="panel settings-card workspace-content" aria-label={p('Team workspace', 'Týmový prostor')} aria-busy={selecting}>{selecting ? <div className="section-loading" role="status"><span className="loading-spinner" aria-hidden="true" />{p('Loading workspace…', 'Načítám prostor…')}</div> : !org || !detail ? <div className="workspace-empty"><span className="workspace-empty-symbol"><Buildings size={38} weight="duotone" aria-hidden="true" /></span><p className="eyebrow">{p('A SPACE FOR YOUR TEAM', 'PROSTOR PRO VÁŠ TÝM')}</p><h2>{p('A healthier routine, together.', 'Zdravější návyky společně.')}</h2><p>{organizations.length ? p('Choose a workspace to see its programs and benefit points.', 'Vyberte prostor pro zobrazení programů a benefitních bodů.') : p('Create a workspace, fund a point pool and invite your team. Joining an existing team? Use your invitation code.', 'Vytvořte prostor, doplňte bodový pool a pozvěte tým. Připojujete se k existujícímu týmu? Použijte kód pozvánky.')}</p>{!organizations.length && <button className="button primary" disabled={busy} onClick={() => { setNewCompany(true); setEditing(null); }}><Plus size={18} aria-hidden="true" />{p('Create your first workspace', 'Vytvořit první prostor')}</button>}</div> : <>
        <div className="workspace-heading"><div><p className="eyebrow">{org.role === 'OWNER' ? p('OWNER', 'VLASTNÍK') : org.role === 'ADMIN' ? p('ADMINISTRATOR', 'SPRÁVCE') : org.role === 'OPERATOR' ? p('OPERATOR', 'OPERÁTOR') : p('MEMBER', 'ČLEN')}</p><h2>{org.name}</h2></div><Buildings size={27} aria-hidden="true" /></div><p className="body-copy workspace-privacy"><ShieldCheck size={17} aria-hidden="true" />{p('Employers see progress, never private running recordings.', 'Zaměstnavatelé vidí plnění, nikdy soukromé záznamy běhu.')}</p>
        {org.role !== 'OPERATOR' && <BusinessPoints key={org.id} organization={org} revision={pointsRevision} busy={busy} run={run} onChanged={() => refresh(org.id)} onError={failed} />}
        {archived && <p className="company-local-note"><Archive size={20} aria-hidden="true" /><span>{p('Archived workspace · history is available for reading. The owner can restore it below.', 'Archivovaný prostor · historie je dostupná ke čtení. Vlastník jej může níže obnovit.')}</span></p>}
        {manage && <div className="company-tabs" aria-label={p('Workspace sections', 'Sekce prostoru')}><button aria-pressed={tab === 'programs'} disabled={busy} onClick={() => setTab('programs')}>{p('Programs', 'Programy')}</button><button aria-pressed={tab === 'members'} disabled={busy} onClick={() => { setTab('members'); setProgram(null); }}>{p('Members & invitations', 'Členové a pozvánky')}</button></div>}
        {tab === 'members' && manage ? <TeamMembers key={`${org.id}-${archived ? 'archived' : 'active'}`} detail={detail} userId={session.user.id} busy={busy || archived} run={run} refresh={() => refresh(org.id)} /> : program ? <BusinessProgram key={`${org.id}-${program.id}-${operatorEnrollment ?? 'own'}`} organization={org} initial={program} userId={session.user.id} busy={busy} run={run} onBack={() => { if (operatorEnrollment) { setDetail(null); setSelectedId(null); } setProgram(null); setOperatorEnrollment(undefined); }} onError={failed} onChanged={() => operatorEnrollment ? list() : refresh(org.id)} operatorEnrollment={operatorEnrollment} onNextCycle={next => { setProgram(next); setEditing(null); setTab('programs'); }} /> : <>
          <div className="company-programs-heading"><div><h3>{p('Team programs', 'Týmové programy')}</h3><p>{p('Choose a program. Move together. Earn points.', 'Vyberte program. Hýbejte se společně. Získejte body.')}</p></div>{manage && !archived && !editing && <button className="button primary" disabled={busy} onClick={() => setEditing('new')}><Plus size={18} aria-hidden="true" />{p('New program', 'Nový program')}</button>}</div>
          {editing && manage && !archived ? <ProgramDraftForm key={editing === 'new' ? `new-${org.id}` : editing.id} initial={editing === 'new' ? null : editing} busy={busy} onSave={saveProgram} onCancel={() => setEditing(null)} /> : <><div className="company-programs">{!activePrograms.length ? <div className="program-empty"><Target size={30} aria-hidden="true" /><p>{archived ? p('No unarchived programs. Retained programs are in the archive below.', 'Žádné nearchivované programy. Zachované programy jsou v archivu níže.') : manage ? p('Prepare one achievable goal, fund it and publish.', 'Připravte dosažitelný cíl, financujte jej a zveřejněte.') : p('No published programs yet. Ask your administrator.', 'Zatím nejsou zveřejněné programy. Požádejte správce.')}</p></div> : activePrograms.map(programCard)}</div>{!!archivedPrograms.length && <details className="section-disclosure" open={archived || undefined}><summary>{p('Archived programs', 'Archivované programy')} · {archivedPrograms.length}</summary><p className="field-hint">{p('Recorded participation and rewards remain available.', 'Zaznamenaná účast a odměny zůstávají dostupné.')}</p><div className="company-programs">{archivedPrograms.map(programCard)}</div></details>}</>}
        </>}
        {manage && <details className="section-disclosure workspace-settings"><summary>{p('Workspace settings & history', 'Nastavení a historie prostoru')}<CaretDown size={17} aria-hidden="true" /></summary><div className="disclosure-content">{!archived && <form key={`${org.id}-${org.version}`} onSubmit={event => { event.preventDefault(); const name = String(new FormData(event.currentTarget).get('company_name')); void run(async () => { await api(companyPath(org.id), { method: 'PATCH', body: { name, version: org.version } }); await refresh(org.id); }); }}><fieldset disabled={busy}><Field label={p('Workspace name', 'Název prostoru')}><input name="company_name" defaultValue={org.name} required minLength={2} maxLength={100} /></Field><button className="button secondary">{p('Rename workspace', 'Přejmenovat prostor')}</button></fieldset></form>}<div className="timeline company-history">{detail.events.map(event => <div className="timeline-item" key={event.id}><ClockCounterClockwise size={16} aria-hidden="true" /><div><time>{date(event.created_at)}</time><p>{event.kind.replaceAll('_', ' ')}</p></div></div>)}</div></div></details>}
        <BusinessWorkspaceLifecycle key={org.id} organization={org} management={detail.management} busy={busy} run={run} onChanged={workspaceChanged} onPrograms={() => { setTab('programs'); setProgram(null); setEditing(null); }} />
      </>}</section></div>
    <details className="company-capabilities section-disclosure"><summary>{p('What works now & what is planned', 'Co funguje nyní a co je v plánu')}</summary><div className="company-capabilities-grid"><div><h3>{p('Available in this prototype', 'Dostupné v tomto prototypu')}</h3><p>{p('Company pools and individual point balances, four voluntary running templates, GPX/FIT checks, automatic point payouts and private operator review. HR sees participation and results; recording details stay with the participant and operator.', 'Firemní pooly a osobní bodové zůstatky, čtyři dobrovolné běžecké šablony, kontrola GPX/FIT, automatická výplata bodů a soukromé operátorské posouzení. HR vidí účast a výsledky; detaily záznamu zůstávají účastníkovi a operátorovi.')}</p></div><div><h3>{p('Separate future integrations', 'Samostatné budoucí integrace')}</h3><p>{p('Pedometer and Health Connect, swimming or non-sport goals, live GPS events, benefit portals and cryptographic privacy proofs are not connected here. Current privacy is enforced by server access controls, not a zero-knowledge proof.', 'Krokoměr a Health Connect, plavání či nesportovní cíle, živé GPS akce, benefitní portály a kryptografické důkazy soukromí zde nejsou propojené. Současné soukromí zajišťuje kontrola přístupu na serveru, nikoli zero-knowledge důkaz.')}</p></div></div></details>
  </div>;
}
