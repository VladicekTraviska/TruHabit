import { useEffect, useRef, useState, type FormEvent } from 'react';
import { ArrowLeft, ArrowRight, CalendarBlank, CheckCircle, Clock, FileArrowUp, Handshake, LockSimple, ShieldCheck, Target, Trophy, Users, Wallet, WarningCircle } from '@phosphor-icons/react';
import { api } from './api';
import { Field } from './components';
import { useLanguage } from './i18n';
import { date } from './types';
import { ConfirmActionDialog } from './ConfirmActionDialog';
import { companyPath, credits, points, templateName, type CompanyPoints, type CompanyProgram, type EnrollmentDetail, type Organization, type ProgramDetail, type PublishInput } from './business-types';
import './business-program-ui.css';

type Runner = (action: () => Promise<void>) => Promise<void>;
export function BusinessProgram({ organization, initial, userId, busy, run, onBack, onChanged, onError, operatorEnrollment, onNextCycle }: {
  organization: Organization; initial: CompanyProgram; userId: string; busy: boolean; run: Runner;
  onBack: () => void; onChanged: () => Promise<void>; onError: (error: unknown) => void; operatorEnrollment?: string;
  onNextCycle: (program: CompanyProgram) => void;
}) {
  const language = useLanguage();
  const p = (en: string, cs: string) => language === 'en' ? en : cs;
  const base = companyPath(organization.id, initial.id);
  const [now, setNow] = useState(Date.now);
  useEffect(() => { const timer = window.setInterval(() => setNow(Date.now()), 1000); return () => window.clearInterval(timer); }, []);
  const [detail, setDetail] = useState<ProgramDetail | null>(null);
  const [own, setOwn] = useState<EnrollmentDetail | null>(null);
  const [pointBalance, setPointBalance] = useState<CompanyPoints | null>(null);
  const [termsAccepted, setTermsAccepted] = useState(false);
  const [nextCycleId] = useState(() => crypto.randomUUID());
  const [joinId] = useState(() => crypto.randomUUID());
  const [removeSource, setRemoveSource] = useState<string | null>(null);
  const [publishing, setPublishing] = useState(false);
  const [confirm, setConfirm] = useState<'join' | 'claim' | 'close' | 'archive' | null>(null);
  const [checkingClosure, setCheckingClosure] = useState(false);
  const generation = useRef(0);
  const serverClock = useRef<{ at: number; observed: number } | null>(null);
  const closureRefreshes = useRef(new Set<string>());
  const active = useRef(true);
  const busyRef = useRef(busy);
  busyRef.current = busy;
  const program = detail?.program ?? initial;
  const pointProgram = !!program.template && program.template !== 'LEGACY';
  const match = program.template === 'EMPLOYER_MATCH';
  const monthly = program.template === 'MONTHLY_BUDGET';
  const pointUnit = pointProgram ? p('points', 'bodů') : p('credits', 'kreditů');
  const isManager = organization.role === 'OWNER' || organization.role === 'ADMIN';
  const workspaceArchived = !!organization.archived_at;
  const canManage = isManager && !workspaceArchived;
  const closure = detail?.closure;
  const getServerNow = () => serverClock.current ? serverClock.current.at + Math.max(0, performance.now() - serverClock.current.observed) : Date.now();

  async function load() {
    const revision = ++generation.current;
    const [result, balance] = await Promise.all([api<ProgramDetail>(base), operatorEnrollment || organization.role === 'OPERATOR' ? Promise.resolve(null) : api<CompanyPoints>(`${companyPath(organization.id)}/points`)]);
    const checked = Date.parse(result.server_now ?? result.closure?.checked_at ?? '');
    const observed = performance.now();
    const enrollment = operatorEnrollment ?? result.current_user_enrollment?.id ?? result.participants.find(item => item.user_id === userId)?.id;
    const evidence = enrollment ? await api<EnrollmentDetail>(`${base}/enrollments/${enrollment}`) : null;
    if (generation.current === revision) { serverClock.current = Number.isFinite(checked) ? { at: checked, observed } : null; setDetail(result); setOwn(evidence); setPointBalance(balance); }
  }
  useEffect(() => { active.current = true; const mounted = generation.current; void load().catch(error => { if (generation.current === mounted + 1) onError(error); }); return () => { active.current = false; generation.current++; }; }, [base]);
  useEffect(() => {
    if (program.state !== 'PUBLISHED' || closure?.allowed || !closure?.available_at || !serverClock.current) return;
    const boundary = Date.parse(closure.available_at);
    const key = base + ':' + closure.available_at;
    if (!Number.isFinite(boundary) || closureRefreshes.current.has(key)) return;
    let cancelled = false;
    let timer: number | undefined;
    function schedule() {
      if (cancelled) return;
      const remaining = boundary - getServerNow() + 1000;
      if (remaining > 0) { timer = window.setTimeout(schedule, Math.min(remaining, 2147483647)); return; }
      if (busyRef.current) { timer = window.setTimeout(schedule, 500); return; }
      closureRefreshes.current.add(key);
      setCheckingClosure(true);
      void load().catch(error => { if (active.current) onError(error); }).finally(() => { if (active.current) setCheckingClosure(false); });
    }
    schedule();
    return () => { cancelled = true; if (timer !== undefined) window.clearTimeout(timer); };
  }, [base, program.state, closure?.allowed, closure?.available_at, closure?.checked_at]);
  async function refresh() { await load(); await onChanged(); }
  function confirmAction() {
    const action = confirm;
    void run(async () => {
      try {
        if (workspaceArchived) throw new Error(p('Restore the workspace before changing its programs.', 'Před změnou programů obnovte tento prostor.'));
        if (action === 'join') await api(`${base}/join`, { method: 'POST', body: { id: joinId, accept_terms: termsAccepted || !(match || monthly) } });
        else if (action === 'claim' && own) await api(`${base}/enrollments/${own.enrollment.id}/claim`, { method: 'POST', body: {} });
        else if (action === 'close') {
          if (closure?.allowed !== true) throw new Error(p('Refresh the program to verify closure eligibility before trying again.', 'Před opakováním obnovte program a ověřte, zda jej lze uzavřít.'));
          await api(`${base}/close`, { method: 'POST', body: { version: program.version } });
        } else if (action === 'archive') await api(`${base}/archive`, { method: 'POST', body: { version: program.version } });
        setConfirm(null); await refresh();
      } catch (error) { setConfirm(null); throw error; }
    });
  }
  function confirmSourceRemoval() {
    if (!removeSource || !own) return;
    const upload = removeSource; const enrollmentId = own.enrollment.id;
    void run(async () => {
      try { await api(`${base}/enrollments/${enrollmentId}/uploads/${upload}/source`, { method: 'DELETE' }); setRemoveSource(null); await refresh(); }
      catch (error) { setRemoveSource(null); throw error; }
    });
  }
  const enrollment = own?.enrollment;
  const participants = detail?.participants ?? [];
  const participantCount = detail?.participant_count ?? participants.length;
  const visibleProgress = enrollment ? [...participants, enrollment] : participants;
  const currentTime = serverClock.current ? getServerNow() : now;
  const uploadOpen = !!program.upload_deadline && currentTime <= new Date(program.upload_deadline).getTime();
  const joinOpen = !!program.ends_at && currentTime < new Date(program.ends_at).getTime();
  const canJoin = !!detail && !workspaceArchived && joinOpen && organization.role !== 'OPERATOR' && program.state === 'PUBLISHED' && !enrollment && !operatorEnrollment && participantCount < program.max_participants;
  const met = enrollment?.assessment === 'MET';
  const rewarded = enrollment?.state === 'REWARDED';
  const closureReasons: Record<string, string> = {
    BUSINESS_UPLOAD_WINDOW_OPEN: p('The agreed upload window is still open.', 'Sjednané období nahrávání ještě běží.'),
    BUSINESS_ACCEPTED_REWARDS_MUST_BE_PAID: p('Accepted rewards are waiting to be paid.', 'Přijaté odměny čekají na vyplacení.'),
    BUSINESS_REVIEW_PENDING: p('A pending operator review is protected until the agreed review deadline.', 'Čekající operátorské posouzení je chráněné do sjednaného termínu.'),
    BUSINESS_PROGRAM_NOT_PUBLISHED: p('This program is not in the published state.', 'Program není ve zveřejněném stavu.'),
    WORKSPACE_ARCHIVED: p('This workspace is archived.', 'Tento prostor je archivovaný.'),
  };
  const closureSteps: Record<string, string> = {
    BUSINESS_UPLOAD_WINDOW_OPEN: p('Keep the program open for participants. It can close after uploads end, or earlier when all places have been rewarded. Before the start, a program with no participants can also close.', 'Nechte program otevřený účastníkům. Lze jej uzavřít po skončení nahrávání nebo dříve po vyplacení všech míst. Před začátkem lze uzavřít i program bez účastníků.'),
    BUSINESS_ACCEPTED_REWARDS_MUST_BE_PAID: p('Ask qualifying participants to claim their earned reward, then refresh the program. An accepted reward does not expire.', 'Požádejte účastníky se splněným cílem o vyzvednutí získané odměny a potom obnovte program. Přijatá odměna nepropadá.'),
    BUSINESS_REVIEW_PENDING: p('An operator can finish the reviews before the deadline. Otherwise wait for that deadline and refresh eligibility. Accepted unpaid rewards still need payment.', 'Operátor může dokončit posouzení před termínem. Jinak vyčkejte do termínu a obnovte podmínky uzavření. Přijaté nevyplacené odměny je stále nutné vyplatit.'),
  };

  const insufficientPledge = match && (pointBalance?.own_available_points ?? 0) < (program.point_stake ?? 0);
  const joinDescription = match ? p('Your earned company points are locked as a voluntary pledge. Success returns them and adds the company bonus. If the goal is unmet when the program closes after its deadlines, the pledge goes to the company pool.', 'Vaše získané firemní body se dobrovolně uzamknou jako garance. Úspěch je vrátí a přidá firemní bonus. Pokud cíl není při uzavření po sjednaných termínech splněný, garance připadne firemnímu poolu.') : monthly ? p('Only this unearned extra bonus declines through the activity window. The qualifying run time fixes the amount; earned points are never reduced. Saved-file demo mode uses the upload admission time.', 'V průběhu období klesá pouze tento nezískaný bonus navíc. Čas vyhovujícího běhu zafixuje částku; získané body se nekrátí. Ukázka se starším souborem používá čas přijetí uploadu.') : p('Join first, then upload one qualifying run. The employer funds the reward; no personal points or credits are charged. Your file stays private.', 'Nejprve se přihlaste, potom nahrajte vyhovující běh. Odměnu hradí firma; vaše body ani kredity se neodečítají. Soubor zůstává soukromý.');
  const currentBonus = program.state !== 'PUBLISHED' ? 0 : monthly && program.starts_at && program.ends_at ? Math.max(0, Math.floor((program.point_reward ?? 0) * Math.min(1, Math.max(0, (Date.parse(program.ends_at) - getServerNow()) / (Date.parse(program.ends_at) - Date.parse(program.starts_at)))))) : program.point_reward ?? 0;
  const TemplateIcon = program.template === 'EVENT' ? Trophy : match ? Handshake : monthly ? CalendarBlank : Target;
  async function nextCycle() { await run(async () => { const next = await api<CompanyProgram>(base + '/next-cycle', { method: 'POST', body: { id: nextCycleId, version: program.version } }); await onChanged(); onNextCycle(next); }); }
  return <section className={`company-program-detail template-${(program.template ?? 'LEGACY').toLowerCase()}`} aria-busy={busy}>
    <button className="button text-button" type="button" disabled={busy} onClick={onBack}><ArrowLeft size={17} aria-hidden="true" />{p('All programs', 'Všechny programy')}</button>
    <div className="program-card-heading company-detail-heading"><div className="company-program-identity"><span className="company-program-symbol"><TemplateIcon size={29} weight="duotone" aria-hidden="true" /></span><div><p className="eyebrow">{templateName(program.template, language)}</p><h3>{program.title}</h3></div></div><span className={`badge company-state state-${program.state.toLowerCase()}`}><span aria-hidden="true" />{program.state === 'DRAFT' ? p('Draft', 'Návrh') : program.state === 'PUBLISHED' ? p('Published', 'Zveřejněno') : program.state === 'CLOSED' ? p('Closed', 'Uzavřeno') : p('Archived', 'Archivováno')}</span></div>
    {workspaceArchived && <p className="company-local-note"><ShieldCheck size={19} aria-hidden="true"/><span>{p('This workspace is archived. Restore it to manage programs. Saved results remain viewable and eligible private source files can still be removed.', 'Tento prostor je archivovaný. Pro správu programů jej obnovte. Uložené výsledky lze dále zobrazit a vypořádané soukromé zdrojové soubory smazat.')}</span></p>}
    <p className="company-local-note"><ShieldCheck size={19} aria-hidden="true" /><span>{pointProgram ? p('Simulated company points. No real payment, personal B2C balance or B2B blockchain transfer.', 'Simulované firemní body. Bez skutečné platby, osobního B2C zůstatku či B2B blockchainového převodu.') : p('Local test credits. No real payment or blockchain transfer. Your personal stake is never charged.', 'Lokální testovací kredity. Bez skutečné platby a blockchainového převodu. Váš osobní vklad se nikdy neúčtuje.')}</span></p>
    <div className="program-metrics"><div><Target size={19} aria-hidden="true" /><strong>{program.target_m / 1000} <small>km</small></strong><span>{p('One qualifying run', 'Jeden vyhovující běh')}</span></div><div><Users size={19} aria-hidden="true" /><strong>{participantCount} / {program.max_participants}</strong><span>{p('Participants', 'Účastníci')}</span></div><div><Trophy size={19} aria-hidden="true" /><strong>{pointProgram ? points(program.point_reward, language) : credits(program.reward_units, language)} <small>{pointUnit}</small></strong><span>{p('Funded reward per participant', 'Financovaná odměna účastníka')}</span></div></div>
    {program.state === 'DRAFT' ? <>
      <p className="body-copy">{pointProgram ? p('This draft has no reserved points. Publication funds the full capacity from the company pool and fixes the dates and template.', 'Návrh zatím nemá rezervované body. Zveřejnění financuje všechna místa z firemního poolu a zafixuje termíny i šablonu.') : p('This draft has no reserved reward. Publishing sets a separate test-credit reward; the original CZK planning amount is preserved.', 'Tento návrh nemá rezervovanou odměnu. Zveřejnění nastaví samostatnou odměnu v testovacích kreditech; původní plánovaná částka v Kč zůstane zachovaná.')}</p>
      {!workspaceArchived && (organization.role === 'OWNER' ? !publishing ? <button className="button primary" disabled={busy || !detail} onClick={() => setPublishing(true)}>{p('Fund & publish program', 'Financovat a zveřejnit program')}<ArrowRight size={18} aria-hidden="true" /></button> : <PublishForm organizationId={organization.id} program={program} busy={busy} getServerNow={getServerNow} onError={onError} onCancel={() => setPublishing(false)} onPublish={body => run(async () => { await api(`${base}/publish`, { method: 'POST', body }); setPublishing(false); await refresh(); })} /> : <p className="field-hint">{pointProgram ? p('The workspace owner approves funding from the company point pool.', 'Financování z firemního bodového poolu potvrzuje vlastník prostoru.') : p('The workspace owner must approve funding from their own available test credits.', 'Financování ze svých dostupných testovacích kreditů musí potvrdit vlastník prostoru.')}</p>)}
      {canManage && <button className="button text-button" disabled={busy || !detail} onClick={() => setConfirm('archive')}>{p('Archive draft', 'Archivovat návrh')}</button>}
    </> : <>
      <ol className="company-lifecycle" aria-label={p('Program progress', 'Průběh programu')}><li className="complete">{p('Funded', 'Financováno')}</li><li className={participantCount ? 'complete' : ''}>{p('Participation', 'Účast')}</li><li className={visibleProgress.some(item => item.assessment === 'MET') ? 'complete' : ''}>{p('Evidence', 'Výsledky')}</li><li className={visibleProgress.some(item => item.state === 'REWARDED') ? 'complete' : ''}>{p('Rewards', 'Odměny')}</li></ol>
      <dl className="company-agreement"><dt>{p('Activity mode', 'Režim aktivity')}</dt><dd>{program.profile === 'REPLAY' ? p('Use a saved GPX/FIT', 'Použít starší GPX/FIT') : p('Record a new run', 'Zaznamenat nový běh')}</dd><dt>{p('Activity window', 'Období běhu')}</dt><dd>{program.profile === 'REPLAY' ? p('Historical recordings allowed', 'Historické záznamy povoleny') : program.starts_at && program.ends_at ? `${date(program.starts_at)} → ${date(program.ends_at)}` : '—'}</dd><dt>{p('Upload deadline', 'Termín nahrání')}</dt><dd>{program.upload_deadline ? date(program.upload_deadline) : '—'}</dd><dt>{p('Review deadline', 'Termín posouzení')}</dt><dd>{program.review_deadline ? date(program.review_deadline) : '—'}</dd></dl>
      <p className="field-hint">{p('The published distance, reward and dates are fixed. An accepted earned reward must be paid before the program can close.', 'Zveřejněná vzdálenost, odměna a termíny jsou pevné. Přijatá získaná odměna musí být vyplacená, než lze program uzavřít.')}</p>
      {isManager && <div className="company-budget-summary"><div><span>{p('Funded budget', 'Financovaný rozpočet')}</span><strong>{credits(program.budget_units, language)} {pointUnit}</strong></div><div><span>{p('Reserved', 'Rezervováno')}</span><strong>{credits(program.reserved_units, language)} {pointUnit}</strong></div><div><span>{p('Paid rewards', 'Vyplacené odměny')}</span><strong>{credits(program.paid_units, language)} {pointUnit}</strong></div></div>}
      {pointProgram && <div className="company-template-summary"><strong>{templateName(program.template, language)}</strong><p>{joinDescription}</p>{match && <p><Handshake size={18} aria-hidden="true"/> {p('Your pledge', 'Vaše garance')}: <strong>{points(program.point_stake, language)} {pointUnit}</strong> · {p('Success returns pledge + bonus', 'Úspěch vrátí garanci a bonus')}: <strong>{points((program.point_stake ?? 0) + (program.point_reward ?? 0), language)} {pointUnit}</strong></p>}{monthly && <p><CalendarBlank size={18} aria-hidden="true"/> {p('Current prospective bonus', 'Aktuální nezískaný bonus')}: <strong>{points(currentBonus, language)} {pointUnit}</strong> · {p('Server-clock estimate; the recorded activity determines the final award.', 'Odhad podle času serveru; konečnou odměnu určí zaznamenaná aktivita.')}</p>}</div>}
      {canJoin && <div className="company-next-step"><h4>{p('Join this voluntary program', 'Přihlaste se do dobrovolného programu')}</h4><p>{joinDescription}</p>{match && <p>{p('Your available company points', 'Vaše volné firemní body')}: <strong>{points(pointBalance?.own_available_points, language)}</strong>{insufficientPledge && <span className="company-publish-budget-warning">{p(' Earn points in an Activity Points or Event program first.', ' Nejprve získejte body v programu Body za aktivitu nebo Týmová akce.')}</span>}</p>}<button className="button primary" disabled={busy || insufficientPledge || (match && !pointBalance)} onClick={() => { setTermsAccepted(false); setConfirm('join'); }}>{p('Review & join program', 'Zkontrolovat a přihlásit se')}<ArrowRight size={18} aria-hidden="true" /></button></div>}
      {!enrollment && program.state === 'PUBLISHED' && !canJoin && !busy && <p className="field-hint">{p('There are no free places or enrollment is unavailable. Refresh to see the latest status.', 'Nejsou volná místa nebo není přihlášení dostupné. Obnovte aktuální stav.')}</p>}
      {program.state === 'PUBLISHED' && !uploadOpen && !met && !rewarded && <p className="company-local-note">{p('The upload window has closed. Accepted rewards remain claimable; a pending review is handled by the operator before the review deadline.', 'Období nahrávání skončilo. Přijaté odměny lze stále vyzvednout; čekající posouzení vyřídí operátor před termínem posouzení.')}</p>}
      {own && <div className="company-own-progress"><div className="company-section-title"><h4>{operatorEnrollment ? p('Private operator review', 'Soukromé operátorské posouzení') : p('Your participation', 'Vaše účast')}</h4><span className={`badge ${rewarded || met ? 'success' : enrollment?.assessment === 'REVIEW_REQUIRED' ? 'attention' : ''}`}>{rewarded ? p('Reward paid', 'Odměna vyplacena') : met ? p('Goal met', 'Cíl splněn') : enrollment?.assessment === 'REVIEW_REQUIRED' ? p('Needs review', 'K posouzení') : enrollment?.state === 'CLOSED' ? p('Closed', 'Uzavřeno') : p('Waiting for a qualifying run', 'Čeká na vyhovující běh')}</span></div>
        {rewarded ? <p className="company-reward-success"><CheckCircle size={25} weight="duotone" aria-hidden="true" />{pointProgram ? p('Points were automatically credited to the participant’s company balance.', 'Body byly automaticky připsány do firemního zůstatku účastníka.') : operatorEnrollment ? p('The reward was credited to the participant’s LOCAL balance.', 'Odměna byla připsána do LOCAL zůstatku účastníka.') : p('Your reward was credited to your LOCAL balance.', 'Odměna byla připsána do vašeho LOCAL zůstatku.')} <strong>{pointProgram ? points(enrollment?.awarded_points, language) : credits(enrollment?.reward_units, language)} {pointUnit}</strong></p> : met && program.state === 'PUBLISHED' && !workspaceArchived ? <div className="company-next-step"><p>{pointProgram ? p('The accepted reward is pending settlement. Retry payment; already settled rewards cannot be paid twice.', 'Přijatá odměna čeká na vypořádání. Zopakujte výplatu; již vyplacená odměna se podruhé nepřipíše.') : operatorEnrollment ? p('This participant qualifies. Pay the reserved reward to their LOCAL account.', 'Účastník splnil podmínky. Vyplaťte rezervovanou odměnu na jeho LOCAL účet.') : p('Your run qualifies. Claim the reserved test-credit reward.', 'Váš běh vyhovuje. Vyzvedněte si rezervovanou odměnu v testovacích kreditech.')}</p><button className="button primary" disabled={busy} onClick={() => setConfirm('claim')}><Trophy size={18} aria-hidden="true" />{operatorEnrollment ? p('Pay participant reward', 'Vyplatit odměnu účastníka') : p('Claim reward', 'Vyzvednout odměnu')}</button></div> : enrollment?.state === 'ENROLLED' && uploadOpen && program.state === 'PUBLISHED' && !workspaceArchived && !operatorEnrollment ? <CompanyEvidenceForm busy={busy} onUpload={(file, session) => run(async () => { await api(`${base}/enrollments/${enrollment.id}/upload${session === '' ? '' : `?session=${Number(session) - 1}`}`, { method: 'POST', file }); await refresh(); })} /> : null}
        {match && rewarded && <p className="company-local-note"><Handshake size={19} aria-hidden="true"/><span>{p('Your locked pledge was also returned:', 'Vaše uzamčená garance byla také vrácena:')} <strong>{points(enrollment?.staked_points, language)} {pointUnit}</strong>. {p('It is separate from the bonus shown above.', 'Je oddělená od bonusu uvedeného výše.')}</span></p>}
        {match && enrollment?.state === 'CLOSED' && (enrollment.staked_points ?? 0) > 0 && <p className="company-local-note"><Handshake size={19} aria-hidden="true"/><span>{p('The goal was unmet at closure. Under the terms you accepted, your pledge went to the company pool:', 'Při uzavření nebyl cíl splněný. Podle přijatých pravidel připadla vaše garance firemnímu poolu:')} <strong>{points(enrollment.staked_points, language)} {pointUnit}</strong>. {p('No personal B2C funds or wages were charged.', 'Osobní B2C prostředky ani mzda se neúčtovaly.')}</span></p>}
        {enrollment?.assessment === 'REVIEW_REQUIRED' && <p className="company-local-note"><Clock size={19} aria-hidden="true" /><span>{p('Unusual data needs a human operator. No settlement is made by this review notice. Your employer cannot access your private recording.', 'Neobvyklá data musí posoudit lidský operátor. Tato zpráva sama garanci nevypořádá. Zaměstnavatel nemá přístup k soukromému záznamu.')}</span></p>}
        {own.uploads.map(upload => <article className="company-evidence" key={upload.id}><div className="program-card-heading"><strong>{p('Saved activity', 'Uložená aktivita')} · {upload.activity.format}</strong><span className={`badge ${upload.decision === 'ACCEPTED' ? 'success' : upload.decision === 'REVIEW_REQUIRED' ? 'attention' : 'company-evidence-rejected'}`}>{upload.decision === 'ACCEPTED' ? p('Counts toward goal', 'Započítáno do cíle') : upload.decision === 'REVIEW_REQUIRED' ? p('Needs review', 'K posouzení') : p('Does not count', 'Nezapočítáno')}</span></div><div className="program-metrics"><div><strong>{new Intl.NumberFormat(language === 'en' ? 'en-GB' : 'cs-CZ', { maximumFractionDigits: 2 }).format(upload.activity.distance_m / 1000)} <small>km</small></strong><span>{p('Distance', 'Vzdálenost')}</span></div><div><strong>{Math.floor(upload.activity.elapsed_seconds / 60)} <small>min</small></strong><span>{p('Duration', 'Trvání')}</span></div><div><strong>{upload.activity.sample_count}</strong><span>{p('Timed samples', 'Časované vzorky')}</span></div></div><p className="field-hint">{date(upload.activity.starts_at)} → {date(upload.activity.ends_at)}</p><p className="body-copy">{({ DISTANCE_NOT_MET: p(`This recording is shorter than the agreed ${program.target_m / 1000} km. Upload a longer run before the deadline.`, `Záznam je kratší než sjednaných ${program.target_m / 1000} km. Nahrajte delší běh před termínem.`), OUTSIDE_ACTIVITY_WINDOW: p('This run took place outside the agreed activity dates. Record a new run in the activity window, or use a saved-file program for historical recordings.', 'Běh proběhl mimo sjednané období. Zaznamenejte nový běh v termínu nebo pro historický záznam použijte program pro starší soubory.'), ACTIVITY_REQUIRES_REVIEW: p('The distance qualifies, but the recording has unusual data. An operator must review it before reward payment.', 'Vzdálenost vyhovuje, ale záznam obsahuje neobvyklá data. Před výplatou jej musí posoudit operátor.'), DISTANCE_AND_WINDOW_MET: p('This run meets the agreed distance and activity dates.', 'Běh splňuje sjednanou vzdálenost i období.') } as Record<string, string>)[upload.reason] ?? upload.reason}</p><p className="field-hint">{p('Manual file: plausibility checks do not prove who ran. This detail stays private to you and the operator.', 'Ruční soubor: kontrola věrohodnosti neprokazuje, kdo běžel. Detail zůstává soukromý pro vás a operátora.')}</p>{upload.activity.checks?.length ? <details className="section-disclosure"><summary>{p('Recorded checks', 'Zaznamenané kontroly')}</summary><ul>{upload.activity.checks.map((check, index) => <li key={index}><strong>{check.outcome}</strong> · {check.code}<p>{check.detail}</p></li>)}</ul></details> : null}{!upload.content_deleted_at ? <div className="form-actions"><a className="button secondary" href={`${base}/enrollments/${own.enrollment.id}/uploads/${upload.id}/source`} download>{p('Download original file', 'Stáhnout původní soubor')}</a>{own.enrollment.user_id === userId && (rewarded || program.state !== 'PUBLISHED') && <button className="button text-button" disabled={busy} onClick={() => setRemoveSource(upload.id)}>{p('Remove source file', 'Smazat zdrojový soubor')}</button>}</div> : <p className="field-hint">{p('Source file removed. The decision record is retained.', 'Zdrojový soubor smazán. Záznam rozhodnutí zůstává.')}</p>}{detail?.is_operator && !workspaceArchived && program.state === 'PUBLISHED' && upload.decision === 'REVIEW_REQUIRED' && <OperatorReviewForm busy={busy} onReview={(accept, reason) => run(async () => { await api(`${base}/enrollments/${own.enrollment.id}/review`, { method: 'POST', body: { upload_id: upload.id, accept, reason } }); await refresh(); })} />}</article>)}
      </div>}
      {isManager && !!participants.length && <section className="company-participants"><h4>{p('Participant progress', 'Plnění účastníků')}</h4><p className="field-hint">{p('Only participation and result status are visible. GPS, heart rate, cadence and uploaded files stay private.', 'Vidíte pouze účast a stav výsledku. GPS, tep, kadence a nahrané soubory zůstávají soukromé.')}</p>{participants.map(item => <div className="company-member-row" key={item.id}><strong>{item.display_name ?? p('Participant', 'Účastník')}</strong><span className="badge">{item.state === 'REWARDED' ? p('Reward paid', 'Odměna vyplacena') : item.assessment === 'MET' ? p('Goal met · reward available', 'Cíl splněn · odměna dostupná') : item.assessment === 'REVIEW_REQUIRED' ? p('Needs review', 'K posouzení') : item.state === 'CLOSED' ? p('Closed', 'Uzavřeno') : p('Enrolled', 'Přihlášen')}</span></div>)}</section>}
      {canManage && program.state === 'PUBLISHED' && <section className={'company-closure-status' + (closure?.allowed ? ' allowed' : '')} aria-label={p('Program closure eligibility', 'Podmínky uzavření programu')}>
        <div className="company-closure-heading">{closure?.allowed ? <CheckCircle size={22} aria-hidden="true"/> : <Clock size={22} aria-hidden="true"/>}<div><h4>{closure?.allowed ? p('Ready to close', 'Připraveno k uzavření') : p('Program remains open', 'Program zůstává otevřený')}</h4><p>{checkingClosure ? p('Checking closure eligibility with the server…', 'Ověřuji podmínky uzavření na serveru…') : closure?.allowed ? p('The server confirmed that this program can close. Unused budget is returned under its published funding rules.', 'Server potvrdil, že lze program uzavřít. Nevyužitý rozpočet se vrátí podle zveřejněných pravidel financování.') : closure?.reason ? closureReasons[closure.reason] ?? p('Closure eligibility has not been confirmed. Refresh the program.', 'Podmínky uzavření nebyly potvrzeny. Obnovte program.') : p('Closure eligibility has not been checked. Refresh the program before closing.', 'Podmínky uzavření nebyly ověřeny. Před uzavřením obnovte program.')}</p></div></div>
        {closure && <dl className="company-closure-counts"><div><dt>{p('Participants', 'Účastníci')}</dt><dd>{closure.participant_count} / {program.max_participants}</dd></div><div><dt>{p('Rewards paid', 'Vyplacené odměny')}</dt><dd>{closure.rewarded_count}</dd></div><div><dt>{p('Accepted rewards unpaid', 'Přijaté nevyplacené odměny')}</dt><dd>{closure.unpaid_rewards}</dd></div><div><dt>{p('Pending reviews', 'Čekající posouzení')}</dt><dd>{closure.pending_reviews}</dd></div></dl>}
        {!closure?.allowed && closure?.reason && closureSteps[closure.reason] && <p className="company-closure-next"><strong>{p('Next step', 'Další krok')}</strong>{closureSteps[closure.reason]}</p>}
        {closure?.available_at && !closure.allowed && <p className="company-closure-deadline"><Clock size={16} aria-hidden="true"/><span><strong>{closure.reason === 'BUSINESS_REVIEW_PENDING' ? p('Review deadline', 'Termín posouzení') : p('Upload deadline', 'Termín nahrání')}: {date(closure.available_at)}</strong><small>{p('This page checks the server once after this time. Closing always requires your confirmation.', 'Tato stránka po tomto čase jednou ověří stav na serveru. Uzavření vždy vyžaduje vaše potvrzení.')}</small></span></p>}
        <div className="form-actions"><button className="button secondary" disabled={busy || checkingClosure || closure?.allowed !== true} onClick={() => setConfirm('close')}>{p('Close & return unused budget', 'Uzavřít a vrátit nevyužitý rozpočet')}</button><button className="button text-button" disabled={busy || checkingClosure} onClick={() => void run(load)}>{p('Refresh eligibility', 'Obnovit podmínky uzavření')}</button></div>
        {closure?.checked_at && <p className="field-hint company-closure-snapshot">{p('Server status checked', 'Stav ověřen na serveru')}: {date(closure.checked_at)}</p>}
      </section>}
      {canManage && program.state === 'CLOSED' && <div className="form-actions"><button className="button secondary" disabled={busy} onClick={() => setConfirm('archive')}>{p('Archive program', 'Archivovat program')}</button></div>}
    </>}
    {monthly && canManage && !!program.published_at && <div className="company-next-cycle"><CalendarBlank size={22} aria-hidden="true"/><div><h4>{p('Prepare the following month', 'Připravit další měsíc')}</h4><p>{p('Creates a separate draft. The owner reviews dates and funds it before publication; nothing renews or charges automatically.', 'Vytvoří samostatný návrh. Vlastník zkontroluje termíny a financuje jej před zveřejněním; nic se neobnovuje ani neúčtuje automaticky.')}</p><button className="button secondary" disabled={busy} onClick={() => void nextCycle()}>{p('Create next monthly draft', 'Vytvořit návrh dalšího měsíce')}</button></div></div>}
    <button className="button text-button" disabled={busy} onClick={() => void run(load)}>{p('Refresh program', 'Obnovit program')}</button>
    {removeSource && own && <ConfirmActionDialog title={p('Remove this private source file?', 'Smazat soukromý zdrojový soubor?')} label={p('Remove file', 'Smazat soubor')} busy={busy} onClose={() => setRemoveSource(null)} onConfirm={confirmSourceRemoval}><p>{p('This removes the original GPX/FIT from TruHabit. The recorded result and reward history remain.', 'Původní GPX/FIT se odstraní z TruHabit. Zaznamenaný výsledek a historie odměny zůstávají.')}</p></ConfirmActionDialog>}
    {confirm && <ConfirmActionDialog title={confirm === 'join' ? p('Join this program?', 'Přihlásit se do tohoto programu?') : confirm === 'claim' ? operatorEnrollment ? p('Pay this participant’s reward?', 'Vyplatit odměnu tohoto účastníka?') : p('Claim your earned reward?', 'Vyzvednout získanou odměnu?') : confirm === 'archive' ? p('Archive this program?', 'Archivovat tento program?') : p('Close this program?', 'Uzavřít tento program?')} label={confirm === 'join' ? p('Join program', 'Přihlásit se do programu') : confirm === 'claim' ? operatorEnrollment ? p('Pay participant reward', 'Vyplatit odměnu účastníka') : p('Claim reward', 'Vyzvednout odměnu') : p('Confirm', 'Potvrdit')} busy={busy} confirmDisabled={workspaceArchived || (confirm === 'join' && ((match || monthly) && !termsAccepted || insufficientPledge)) || (confirm === 'close' && (checkingClosure || closure?.allowed !== true))} onClose={() => setConfirm(null)} onConfirm={confirmAction}><p><strong>{program.title}</strong></p><p>{confirm === 'join' ? joinDescription : confirm === 'claim' ? pointProgram ? p('Credits the accepted company-point reward once. A successful Employer Match also returns the locked pledge. No real money or B2B blockchain transfer is used.', 'Jednou připíše přijatou odměnu ve firemních bodech. Úspěšná Spoluúčast firmy také vrátí uzamčenou garanci. Bez skutečných peněz a B2B blockchainového převodu.') : p('The reserved LOCAL test reward will be credited once to the participant account. No real money or blockchain transfer is used.', 'Rezervovaná LOCAL testovací odměna bude jednou připsána na účet účastníka. Bez skutečných peněz a blockchainového převodu.') : confirm === 'archive' ? p('The program leaves the active list. Its saved history is preserved.', 'Program opustí aktivní seznam. Uložená historie zůstane zachovaná.') : p('Closes enrollment and uploads and returns unused budget. Earned rewards and pending reviews are protected. Employer Match settles an unmet pledge into the company pool; other points templates do not charge employees.', 'Uzavře přihlašování a nahrávání a vrátí nevyužitý rozpočet. Získané odměny a čekající posouzení jsou chráněné. Spoluúčast firmy vypořádá nesplněnou garanci do firemního poolu; ostatní bodové šablony zaměstnanci nic neodečítají.')}</p>{confirm === 'join' && (match || monthly) && <label className="company-terms-consent"><input type="checkbox" checked={termsAccepted} disabled={busy} onChange={event => setTermsAccepted(event.target.checked)}/><span>{match ? p('I voluntarily accept locking my company points and the success/failure pledge rules.', 'Dobrovolně souhlasím s uzamčením svých firemních bodů a pravidly garance při úspěchu i neúspěchu.') : p('I voluntarily accept the declining extra bonus. My already earned points remain unchanged.', 'Dobrovolně souhlasím s klesajícím bonusem navíc. Moje již získané body zůstávají zachované.')}</span></label>}</ConfirmActionDialog>}
  </section>;
}

function localInput(value: number) { const date = new Date(value); return new Date(value - date.getTimezoneOffset() * 60000).toISOString().slice(0, 16); }
function PublishForm({ organizationId, program, busy, getServerNow, onError, onCancel, onPublish }: {
  organizationId: string; program: CompanyProgram; busy: boolean; getServerNow: () => number; onError: (error: unknown) => void;
  onCancel: () => void; onPublish: (body: PublishInput) => Promise<void>;
}) {
  const language = useLanguage(); const p = (en: string, cs: string) => language === 'en' ? en : cs;
  const pointProgram = !!program.template && program.template !== 'LEGACY';
  const monthly = program.template === 'MONTHLY_BUDGET';
  const windowDays = monthly ? 32 : 30;
  const unit = pointProgram ? p('points', 'bodů') : p('credits', 'kreditů');
  const maxBudget = pointProgram ? 10_000_000 * 1e6 : 1_000_000_000;
  const defaultStart = program.cycle_starts_at ? Date.parse(program.cycle_starts_at) : getServerNow() + 10 * 60000;
  const defaultEnd = program.cycle_ends_at ? Date.parse(program.cycle_ends_at) : defaultStart + (monthly ? 31 : 1) * 86400000;
  const [profile, setProfile] = useState<'LIVE' | 'REPLAY'>('LIVE');
  const [reward, setReward] = useState(String(pointProgram ? program.point_reward ?? 100 : 5));
  const [starts, setStarts] = useState(() => localInput(defaultStart));
  const [ends, setEnds] = useState(() => localInput(defaultEnd));
  const [cutoff, setCutoff] = useState(() => localInput(defaultEnd + 3600000));
  const [reviewDeadline, setReviewDeadline] = useState(() => localInput(defaultEnd + 2 * 3600000));
  const [confirmation, setConfirmation] = useState<PublishInput | null>(null);
  const [validation, setValidation] = useState<{ field: string; message: string }[]>([]);
  const [available, setAvailable] = useState<number | null>(null);
  const [fundsLoading, setFundsLoading] = useState(true);
  const [fundsFailed, setFundsFailed] = useState(false);
  const fundsActive = useRef(true); const fundsRevision = useRef(0);
  const heading = useRef<HTMLHeadingElement>(null);
  const validationBox = useRef<HTMLDivElement>(null);
  const units = Math.round(Number(reward) * 1e6);
  const budget = Number.isFinite(units) ? units * program.max_participants : 0;
  const maximumWholeReward = Math.min(pointProgram ? 100000 : 50, Math.floor(maxBudget / 1e6 / program.max_participants));
  const fieldInvalid = (field: string) => validation.some(item => item.field === field);
  const fromTime = (value: string, milliseconds: number) => { const time = Date.parse(value); return Number.isFinite(time) ? localInput(time + milliseconds) : undefined; };
  const minimumStart = localInput(Math.ceil((getServerNow() + 5 * 60000) / 60000) * 60000);
  const maximumStart = localInput(getServerNow() + 30 * 86400000);
  useEffect(() => { if (confirmation) heading.current?.focus(); }, [confirmation]);
  useEffect(() => { if (validation.length) validationBox.current?.focus(); }, [validation]);

  async function loadFunds() {
    const revision = ++fundsRevision.current;
    setFundsLoading(true); setFundsFailed(false);
    try {
      const balance = pointProgram ? { available: (await api<CompanyPoints>(companyPath(organizationId) + '/points')).pool_available_points! * 1e6 } : await api<{ available: number }>('/api/prototype/local/balance');
      if (!Number.isFinite(balance.available)) throw new Error(p('The funding balance could not be read.', 'Zůstatek pro financování se nepodařilo přečíst.'));
      if (fundsActive.current && fundsRevision.current === revision) setAvailable(balance.available);
    } catch (error) {
      if (fundsActive.current && fundsRevision.current === revision) { setFundsFailed(true); onError(error); }
    } finally { if (fundsActive.current && fundsRevision.current === revision) setFundsLoading(false); }
  }
  useEffect(() => { fundsActive.current = true; void loadFunds(); return () => { fundsActive.current = false; fundsRevision.current++; }; }, []);
  function changeProfile(value: 'LIVE' | 'REPLAY') {
    setProfile(value); setValidation([]);
    if (value === 'LIVE') {
      const at = getServerNow();
      setStarts(localInput(at + 10 * 60000)); setEnds(localInput(at + (monthly ? 31 : 1) * 86400000 + 10 * 60000));
      setCutoff(localInput(at + (monthly ? 31 : 1) * 86400000 + 70 * 60000)); setReviewDeadline(localInput(at + (monthly ? 31 : 1) * 86400000 + 130 * 60000));
    }
  }
  function terms(): PublishInput {
    const iso = (value: string) => { const time = Date.parse(value); return Number.isFinite(time) ? new Date(time).toISOString() : ''; };
    const at = getServerNow();
    return { version: program.version, reward_units: units, profile,
      starts_at: profile === 'REPLAY' ? new Date(at).toISOString() : iso(starts),
      ends_at: profile === 'REPLAY' ? new Date(at + 10 * 60000).toISOString() : iso(ends),
      upload_deadline: profile === 'REPLAY' ? new Date(at + 10 * 60000).toISOString() : iso(cutoff),
      review_deadline: profile === 'REPLAY' ? new Date(at + 15 * 60000).toISOString() : iso(reviewDeadline) };
  }
  function validate(body: PublishInput) {
    const errors: { field: string; message: string }[] = [];
    const required = body.reward_units * program.max_participants;
    const at = getServerNow();
    if (!Number.isSafeInteger(body.reward_units) || body.reward_units < 1_000_000 || body.reward_units > (pointProgram ? 100000 * 1e6 : 50_000_000))
      errors.push({ field: 'reward', message: pointProgram ? p('Choose 1–100,000 whole company points per participant in the draft.', 'V návrhu zvolte 1–100 000 celých firemních bodů na účastníka.') : p('Choose a reward between 1 and 50 LOCAL test credits per participant.', 'Zvolte odměnu od 1 do 50 LOCAL testovacích kreditů na účastníka.') });
    if (required > maxBudget)
      errors.push({ field: 'budget', message: pointProgram ? p('The total budget exceeds 10,000,000 points. Reduce the reward or number of places in the draft.', 'Celkový rozpočet překračuje 10 000 000 bodů. V návrhu snižte odměnu nebo počet míst.') : p('The total budget exceeds 1,000 credits. Reduce the reward or edit the draft to reduce its maximum participants.', 'Celkový rozpočet překračuje 1 000 kreditů. Snižte odměnu nebo v návrhu snižte maximální počet účastníků.') });
    if (available !== null && Number.isFinite(required) && required > available)
      errors.push({ field: 'funds', message: p('You need ', 'Potřebujete ') + credits(required, language) + ' ' + unit + p('; the last balance read shows ', '; poslední načtený zůstatek je ') + credits(available, language) + ' ' + unit + (pointProgram ? p('. Add demo points to the company pool or edit the draft budget.', '. Doplňte demo body do firemního poolu nebo upravte rozpočet návrhu.') : p('. Add LOCAL test credits in Challenges, reduce the reward, or refresh the balance if it changed.', '. Přidejte LOCAL testovací kredity ve Výzvách, snižte odměnu nebo při změně obnovte zůstatek.')) });
    if (body.version !== program.version)
      errors.push({ field: 'version', message: p('This draft changed. Go back to settings and review its current terms.', 'Návrh se změnil. Vraťte se k nastavení a zkontrolujte aktuální podmínky.') });
    if (body.profile === 'LIVE') {
      const start = Date.parse(body.starts_at); const end = Date.parse(body.ends_at);
      const upload = Date.parse(body.upload_deadline); const review = Date.parse(body.review_deadline);
      if (![start, end, upload, review].every(Number.isFinite)) {
        errors.push({ field: 'dates', message: p('Enter a valid start, end, upload deadline and review deadline.', 'Vyplňte platný začátek, konec, termín nahrání i termín posouzení.') });
      } else {
        if (start < at + 5 * 60000) errors.push({ field: 'starts', message: p('The start must still be at least five minutes away when you confirm publication. Choose ', 'Začátek musí být i při potvrzení zveřejnění nejméně za pět minut. Zvolte ') + date(new Date(Math.ceil((at + 5 * 60000) / 60000) * 60000).toISOString()) + p(' or later.', ' nebo později.') });
        if (start > at + 30 * 86400000) errors.push({ field: 'starts', message: p('The program must start within the next 30 days.', 'Program musí začínat během příštích 30 dnů.') });
        if (end <= start || end > start + windowDays * 86400000) errors.push({ field: 'ends', message: p('End after the start, within the maximum activity window of ', 'Konec musí být po začátku; období běhu může trvat nejvýše ') + windowDays + p(' days.', ' dnů.') });
        if (upload < end || upload > end + 7 * 86400000) errors.push({ field: 'cutoff', message: p('The upload deadline must be at or after the activity end and no more than seven days later.', 'Termín nahrání musí být při konci období běhu nebo později, nejvýše však o sedm dnů.') });
        if (review <= upload || review > upload + 7 * 86400000) errors.push({ field: 'review', message: p('The review deadline must be after the upload deadline and no more than seven days later.', 'Termín posouzení musí být po termínu nahrání, nejvýše však o sedm dnů.') });
      }
    }
    return errors;
  }
  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); const body = terms(); const errors = validate(body); setValidation(errors);
    if (!errors.length) setConfirmation(body);
  }
  function publish() {
    if (!confirmation) return;
    const errors = validate(confirmation); setValidation(errors);
    if (!errors.length) void onPublish(confirmation);
  }
  const balance = <div className="company-funding-balance" aria-busy={fundsLoading}><div><Wallet size={21} aria-hidden="true"/><span>{pointProgram ? p('Available company pool points', 'Volné body ve firemním poolu') : p('Available LOCAL test credits', 'Volné LOCAL testovací kredity')}<strong>{available === null ? '—' : credits(available, language)}</strong></span></div><button type="button" className="button text-button" disabled={busy || fundsLoading} onClick={() => void loadFunds()}>{fundsLoading ? p('Checking balance…', 'Kontroluji zůstatek…') : p('Refresh balance', 'Obnovit zůstatek')}</button><p>{fundsFailed ? p('The balance check failed. Any displayed amount is the previous snapshot; the server will check funding again.', 'Kontrola zůstatku selhala. Zobrazená částka případně vychází z předchozího snímku; server financování znovu ověří.') : p('A balance snapshot. The server verifies the funding balance again when publishing.', 'Snímek zůstatku. Server při zveřejnění znovu ověří zůstatek pro financování.')}</p></div>;
  return <div className="company-program program-form company-publish-form">
    {validation.length > 0 && <div className="company-publish-validation" role="alert" tabIndex={-1} ref={validationBox}><WarningCircle size={21} aria-hidden="true"/><div><strong>{p('Update these settings before publishing', 'Před zveřejněním upravte tato nastavení')}</strong><ul>{validation.map((item, index) => <li key={item.field + index}>{item.message}</li>)}</ul></div></div>}
    {!confirmation ? <form onSubmit={submit}>
      <h4>{pointProgram ? p('Fund from the company point pool', 'Financovat z firemního bodového poolu') : p('Fund with LOCAL test credits', 'Financovat LOCAL testovacími kredity')}</h4>
      <fieldset disabled={busy}>
        {balance}
        {pointProgram ? <p className="company-template-policy"><strong>{templateName(program.template, language)} · {points(program.point_reward, language)} {unit}</strong><br/>{p('The point amount is fixed in the draft. Go back to the program list to edit the draft before publishing.', 'Počet bodů je nastavený v návrhu. Před zveřejněním lze návrh upravit v seznamu programů.')}</p> : <Field label={p('Test reward per participant (credits)', 'Testovací odměna účastníka (kredity)')} hint={p('1–50 credits per participant. The complete budget must stay within 1,000 credits.', '1–50 kreditů na účastníka. Celkový rozpočet musí zůstat do 1 000 kreditů.')}><input type="number" min={1} max={50} step={1} required value={reward} aria-invalid={fieldInvalid('reward')} onChange={event => setReward(event.target.value)}/></Field>}
        <Field label={p('Activity mode', 'Režim aktivity')}><select value={profile} onChange={event => changeProfile(event.target.value as 'LIVE' | 'REPLAY')}><option value="LIVE">{p('Record a new run', 'Zaznamenat nový běh')}</option><option value="REPLAY">{p('Use a saved GPX/FIT', 'Použít starší GPX/FIT')}</option></select></Field>
        <p className="field-hint">{profile === 'REPLAY' ? p('Historical activity dates are allowed. The server sets an upload deadline 10 minutes after publication and a review deadline 15 minutes after publication.', 'Historická data aktivity jsou povolena. Server nastaví termín nahrání na 10 minut a posouzení na 15 minut od zveřejnění.') : p('The activity must fall inside the agreed start and end. Times below are checked again when you confirm publication.', 'Aktivita musí spadat mezi sjednaný začátek a konec. Níže uvedené termíny se znovu kontrolují při potvrzení zveřejnění.')}</p>
        {profile === 'LIVE' && <div className="company-form-grid">
          <Field label={p('Program starts', 'Začátek programu')} hint={p('At least five minutes after confirmation; within the next 30 days.', 'Nejméně pět minut po potvrzení; během příštích 30 dnů.')}><input type="datetime-local" required min={minimumStart} max={maximumStart} value={starts} aria-invalid={fieldInvalid('starts') || fieldInvalid('dates')} onChange={event => setStarts(event.target.value)}/></Field>
          <Field label={p('Activity window ends', 'Konec období běhu')} hint={p('After the start, at most ', 'Po začátku, nejvýše ') + windowDays + p(' days later.', ' dnů později.')}><input type="datetime-local" required min={fromTime(starts, 60000)} max={fromTime(starts, windowDays * 86400000)} value={ends} aria-invalid={fieldInvalid('ends') || fieldInvalid('dates')} onChange={event => setEnds(event.target.value)}/></Field>
          <Field label={p('Upload deadline', 'Termín nahrání')} hint={p('At or after the activity end, no more than seven days later.', 'Při konci období běhu nebo později, nejvýše o sedm dnů.')}><input type="datetime-local" required min={fromTime(ends, 0)} max={fromTime(ends, 7 * 86400000)} value={cutoff} aria-invalid={fieldInvalid('cutoff') || fieldInvalid('dates')} onChange={event => setCutoff(event.target.value)}/></Field>
          <Field label={p('Review deadline', 'Termín posouzení')} hint={p('After uploads end, no more than seven days later.', 'Po termínu nahrání, nejvýše o sedm dnů.')}><input type="datetime-local" required min={fromTime(cutoff, 60000)} max={fromTime(cutoff, 7 * 86400000)} value={reviewDeadline} aria-invalid={fieldInvalid('review') || fieldInvalid('dates')} onChange={event => setReviewDeadline(event.target.value)}/></Field>
        </div>}
        <p className="field-hint">{p('Dates use your device time zone:', 'Termíny používají časové pásmo zařízení:')} {Intl.DateTimeFormat().resolvedOptions().timeZone}</p>
        <p className="program-budget"><span>{pointProgram ? p('Total company points to reserve', 'Celkem firemních bodů k rezervaci') : p('Total credits to reserve', 'Celkem kreditů k rezervaci')}</span><strong>{credits(budget, language)} {unit}</strong></p>
        {budget > maxBudget && <p className="company-publish-budget-warning">{maximumWholeReward >= 1 ? p('For this many participants, choose at most ', 'Pro tento počet účastníků zvolte nejvýše ') + maximumWholeReward + ' ' + unit + p(' each, or reduce the participant limit in the draft.', ' na osobu nebo v návrhu snižte počet účastníků.') : p('Even one credit per participant exceeds the budget cap. Edit the draft to reduce its maximum participants to 1,000 or fewer.', 'I jeden kredit na účastníka překračuje limit rozpočtu. V návrhu snižte maximální počet účastníků na 1 000 nebo méně.')}</p>}
        <p className="body-copy">{pointProgram ? p('This reserves points from the company pool, without charging your personal account. The owner can add simulated points in the pool dashboard above.', 'Rezervuje body z firemního poolu bez odečtení z vašeho osobního účtu. Vlastník může doplnit simulované body ve firemním přehledu výše.') : p('This reserves your available LOCAL credits. Get test credits in Challenges if needed, then refresh the balance here. This funding is separate from the CZK planning estimate.', 'Tato částka se rezervuje z vašich volných LOCAL kreditů. Podle potřeby získejte testovací kredity ve Výzvách a zde obnovte zůstatek. Financování je oddělené od plánovaného odhadu v Kč.')}</p>
        <div className="form-actions"><button className="button primary" disabled={fundsLoading}>{p('Review funding', 'Zkontrolovat financování')}<ArrowRight size={17} aria-hidden="true"/></button><button className="button text-button" type="button" onClick={onCancel}>{p('Cancel', 'Zrušit')}</button></div>
      </fieldset>
    </form> : <>
      <h4 tabIndex={-1} ref={heading}>{p('Confirm program funding', 'Potvrďte financování programu')}</h4>
      <dl className="company-agreement"><dt>{p('Goal', 'Cíl')}</dt><dd>{program.target_m / 1000} km</dd><dt>{p('Maximum participants', 'Maximum účastníků')}</dt><dd>{program.max_participants}</dd><dt>{p('Reward per participant', 'Odměna účastníka')}</dt><dd>{credits(confirmation.reward_units, language)} {unit}</dd><dt>{p('Total reserved', 'Celkem rezervováno')}</dt><dd>{credits(confirmation.reward_units * program.max_participants, language)} {unit}</dd><dt>{p('Activity mode', 'Režim aktivity')}</dt><dd>{confirmation.profile === 'REPLAY' ? p('Historical recordings allowed', 'Historické záznamy povoleny') : p('New run in the agreed window', 'Nový běh ve sjednaném období')}</dd><dt>{p('Program starts', 'Začátek programu')}</dt><dd>{confirmation.profile === 'REPLAY' ? p('At publication', 'Při zveřejnění') : date(confirmation.starts_at)}</dd><dt>{p('Activity window ends', 'Konec období běhu')}</dt><dd>{confirmation.profile === 'REPLAY' ? p('10 minutes after publication', '10 minut od zveřejnění') : date(confirmation.ends_at)}</dd><dt>{p('Upload deadline', 'Termín nahrání')}</dt><dd>{confirmation.profile === 'REPLAY' ? p('10 minutes after publication', '10 minut od zveřejnění') : date(confirmation.upload_deadline)}</dd><dt>{p('Review deadline', 'Termín posouzení')}</dt><dd>{confirmation.profile === 'REPLAY' ? p('15 minutes after publication', '15 minut od zveřejnění') : date(confirmation.review_deadline)}</dd></dl>
      {balance}
      <p className="body-copy">{pointProgram ? p('Publication reserves the company pool budget and fixes the template and dates. Accepted runs pay automatically. Unused points return to the company pool. Employees explicitly accept Match or Monthly terms before joining.', 'Zveřejnění rezervuje firemní rozpočet a zafixuje šablonu i termíny. Přijaté běhy se vyplácejí automaticky. Nevyužité body se vrátí do firemního poolu. Zaměstnanci před účastí výslovně přijmou podmínky Spoluúčasti nebo Měsíčního bonusu.') : p('Publishing locks this test budget and freezes the terms. Members join voluntarily. Unused credits return to you after closure; an accepted reward does not expire.', 'Zveřejnění uzamkne testovací rozpočet a zafixuje podmínky. Členové se přihlašují dobrovolně. Nevyužité kredity se po uzavření vrátí; přijatá odměna nepropadá.')}</p>
      <div className="form-actions"><button className="button primary" disabled={busy || fundsLoading} onClick={publish}>{p('Confirm & publish', 'Potvrdit a zveřejnit')}</button><button className="button secondary" disabled={busy} onClick={() => { setConfirmation(null); setValidation([]); }}>{p('Back to settings', 'Zpět k nastavení')}</button></div>
    </>}
  </div>;
}

function CompanyEvidenceForm({ busy, onUpload }: { busy: boolean; onUpload: (file: File, session: string) => Promise<void> }) {
  const language = useLanguage(); const p = (en: string, cs: string) => language === 'en' ? en : cs;
  const [file, setFile] = useState<File | null>(null); const [session, setSession] = useState('');
  const [invalid, setInvalid] = useState('');
  const fit = !!file && /\.fit$/i.test(file.name);
  function submit(event: FormEvent<HTMLFormElement>) { event.preventDefault(); if (file) void onUpload(file, fit ? session : ''); }
  return <form className="company-upload" onSubmit={submit}>
    <div className="company-upload-heading"><span><FileArrowUp size={25} weight="duotone" aria-hidden="true" /></span><div><h4>{p('Submit your run', 'Nahrajte svůj běh')}</h4><p>{p('We check distance, dates and recording consistency. A short or out-of-window run does not charge your balance.', 'Zkontrolujeme vzdálenost, datum a konzistenci záznamu. Krátký běh ani běh mimo termín neodečte váš zůstatek.')}</p></div></div>
    <fieldset disabled={busy}>
      <Field label={p('GPX or FIT file', 'Soubor GPX nebo FIT')} hint={p('Private upload · up to 16 MiB', 'Soukromý upload · do 16 MiB')}>
        <input name="activity_file" type="file" accept=".gpx,.fit" required onChange={event => {
          const selected = event.target.files?.[0] ?? null;
          setSession('');
          if (selected && selected.size > 16 * 1024 * 1024) {
            setFile(null); setInvalid(p('The file exceeds 16 MiB.', 'Soubor překračuje 16 MiB.')); event.target.value = '';
          } else if (selected && !/\.(gpx|fit)$/i.test(selected.name)) {
            setFile(null); setInvalid(p('Choose an original GPX or FIT activity file.', 'Vyberte původní soubor aktivity GPX nebo FIT.')); event.target.value = '';
          } else { setFile(selected); setInvalid(''); }
        }} />
      </Field>
      {file && <p className="company-selected-file"><CheckCircle size={21} aria-hidden="true" /><span><strong>{file.name}</strong><small>{new Intl.NumberFormat(language === 'en' ? 'en-GB' : 'cs-CZ', { maximumFractionDigits: 2 }).format(file.size / (1024 * 1024))} MiB · {p('Ready to check', 'Připraveno ke kontrole')}</small></span></p>}
      {invalid && <p role="alert">{invalid}</p>}
      {fit && <details className="company-fit-session"><summary>{p('FIT with multiple sessions?', 'FIT s více aktivitami?')}</summary><Field label={p('Activity number', 'Číslo aktivity')} hint={p('Leave blank for a single activity. For multiple activities, choose 1, 2, and so on.', 'U jedné aktivity nechte prázdné. U více aktivit zvolte 1, 2 a tak dále.')}><input name="fit_activity" type="number" min={1} max={1001} step={1} value={session} onChange={event => setSession(event.target.value)} /></Field></details>}
      <div className="company-upload-actions"><span><LockSimple size={16} aria-hidden="true" />{p('Your original recording stays private', 'Váš původní záznam zůstává soukromý')}</span><button className="button primary" disabled={!file || busy}><FileArrowUp size={18} aria-hidden="true" />{busy ? p('Checking…', 'Kontroluji…') : p('Upload & check run', 'Nahrát a zkontrolovat běh')}</button></div>
    </fieldset>
  </form>;
}

function OperatorReviewForm({ busy, onReview }: { busy: boolean; onReview: (accept: boolean, reason: string) => Promise<void> }) {
  const language = useLanguage(); const p = (en: string, cs: string) => language === 'en' ? en : cs;
  function submit(event: FormEvent<HTMLFormElement>) { event.preventDefault(); const data = new FormData(event.currentTarget); void onReview(data.get('accept') === 'true', String(data.get('reason')).trim()); }
  return <form className="company-review-form" onSubmit={submit}><h4>{p('Operator decision', 'Rozhodnutí operátora')}</h4><fieldset disabled={busy}><Field label={p('Decision', 'Rozhodnutí')}><select name="accept"><option value="true">{p('Accept qualifying evidence', 'Přijmout vyhovující podklad')}</option><option value="false">{p('Reject this evidence', 'Zamítnout podklad')}</option></select></Field><Field label={p('Reason saved in the private history', 'Důvod uložený v soukromé historii')}><textarea name="reason" minLength={5} maxLength={500} required rows={3} /></Field><p className="field-hint">{p('A review cannot override the agreed distance or dates.', 'Posouzení nemůže obejít sjednanou vzdálenost a datum.')}</p><button className="button secondary">{p('Save reviewed decision', 'Uložit posouzení')}</button></fieldset></form>;
}
