import { useId, type ReactNode } from 'react';
import { ArrowRight, CheckCircle, Clock, FileText, Info, LockKey, ShieldCheck, WarningCircle } from '@phosphor-icons/react';
import { useLanguage } from './i18n';
import { date } from './types';
import './challenge-guidance.css';

export interface GuidanceChallenge {
  target_m: number;
  profile: 'LIVE' | 'REPLAY';
  network: 'LOCAL' | 'DEVNET';
  starts_at: string;
  ends_at: string;
  state: string;
}

export interface GuidanceUpload {
  activity: { distance_m: number; starts_at: string; ends_at: string };
  goal_result: string;
  decision: string;
  reason: string;
}

export interface ActivityExplanationProps {
  challenge: GuidanceChallenge;
  upload: GuidanceUpload;
  onHistoricalReplay?: () => void;
}

export interface NextStepHintProps {
  state: string;
  profile: 'LIVE' | 'REPLAY';
  network: 'LOCAL' | 'DEVNET';
  assessment: string;
  startsAt: string;
  endsAt: string;
  uploadDeadline: string;
  refundAfter: string;
  now?: number;
  walletLinked: boolean;
  signedPending: boolean;
  activationExpired: boolean;
  lastUpload?: Pick<GuidanceUpload, 'decision' | 'reason' | 'goal_result'>;
  canOperate?: boolean;
}

type Tone = 'success' | 'neutral' | 'attention';
type PickText = (english: string, czech: string) => string;

function displayedDate(value: string, p: PickText): string {
  return Number.isFinite(Date.parse(value)) ? date(value) : p('Time not available', 'Čas není dostupný');
}

function fundsDescription(state: string, network: 'LOCAL' | 'DEVNET', p: PickText): { label: string; description: string; tone: Tone } {
  const local = network === 'LOCAL';
  if (['REFUNDED', 'CANCELLED', 'EXPIRED'].includes(state)) {
    return {
      label: p('Challenge stake returned', 'Vklad výzvy vrácen'),
      description: local
        ? p('The simulation credits were released back to the challenge owner’s available balance.', 'Simulační kredity byly uvolněny zpět do volného zůstatku vlastníka výzvy.')
        : p('The recorded challenge status confirms the test-token return. Its transaction is in the history.', 'Zaznamenaný stav výzvy potvrzuje vrácení testovacích tokenů. Transakci najdete v historii.'),
      tone: 'success',
    };
  }
  if (state === 'FORFEITED') {
    return {
      label: p('Challenge stake forfeited', 'Vklad výzvy propadl'),
      description: local
        ? p('The simulation records show a transfer to the simulated failure recipient.', 'Simulační záznamy ukazují převod simulovanému příjemci při neúspěchu.')
        : p('The recorded challenge status shows a transfer to the agreed failure recipient. Check its transaction in the history.', 'Zaznamenaný stav výzvy ukazuje převod sjednanému příjemci při neúspěchu. Transakci ověřte v historii.'),
      tone: 'attention',
    };
  }
  if (state === 'ACTIVE') {
    return {
      label: local ? p('Challenge credits still locked', 'Kredity výzvy jsou stále uzamčené') : p('Stake return not recorded yet', 'Vrácení vkladu zatím není zaznamenané'),
      description: local
        ? p('Reading a file does not return credits. The challenge still needs a settlement action.', 'Načtení souboru nevrací kredity. Výzva ještě potřebuje akci pro vypořádání.')
        : p('The saved state still shows an active stake. Reading a file does not transfer tokens; settlement needs its own confirmed transaction.', 'Uložený stav stále ukazuje aktivní vklad. Načtení souboru nepřevádí tokeny; vypořádání potřebuje vlastní potvrzenou transakci.'),
      tone: 'neutral',
    };
  }
  return {
    label: p('Settlement not confirmed', 'Vypořádání není potvrzené'),
    description: p('Check the challenge status and any pending transaction before taking another action.', 'Před další akcí ověřte stav výzvy a případnou čekající transakci.'),
    tone: 'neutral',
  };
}

function EvidenceStage({ number, title, description, tone }: { number: number; title: string; description: string; tone: Tone }) {
  return <li className={'activity-explanation-stage tone-' + tone}>
    <span className="activity-explanation-stage-number" aria-hidden="true">{number}</span>
    <div><strong>{title}</strong><p>{description}</p></div>
  </li>;
}

export function ActivityExplanation({ challenge: c, upload: u, onHistoricalReplay }: ActivityExplanationProps) {
  const language = useLanguage();
  const p: PickText = (en, cs) => language === 'en' ? en : cs;
  const heading = useId();
  const a = u.activity;
  const accepted = u.decision === 'ACCEPTED';
  const review = u.decision === 'REVIEW_REQUIRED';
  const outside = !accepted && !review && c.profile === 'LIVE' && u.reason === 'OUTSIDE_ACTIVITY_WINDOW';
  const shorter = !accepted && !review && u.reason === 'DISTANCE_NOT_MET';
  const distanceMet = a.distance_m >= c.target_m;
  const start = Date.parse(a.starts_at);
  const end = Date.parse(a.ends_at);
  const requiredStart = Date.parse(c.starts_at);
  const requiredEnd = Date.parse(c.ends_at);
  const windowKnown = [start, end, requiredStart, requiredEnd].every(Number.isFinite);
  const inWindow = c.profile === 'REPLAY' || (windowKnown && start >= requiredStart && end <= requiredEnd);
  const number = (metres: number) => new Intl.NumberFormat(language === 'en' ? 'en-GB' : 'cs-CZ', { maximumFractionDigits: 2 }).format(metres / 1000) + ' km';
  const funds = fundsDescription(c.state, c.network, p);
  const title = accepted
    ? p('This run counts toward your challenge.', 'Tento běh se počítá do vaší výzvy.')
    : review
      ? p('The file was read. A person needs to review this run.', 'Soubor byl načten. Tento běh musí posoudit člověk.')
      : outside
        ? p('The file was read, but this run happened outside your challenge dates.', 'Soubor byl načten, ale běh proběhl mimo termín vaší výzvy.')
        : shorter
          ? p('The file was read, but this run is shorter than your goal.', 'Soubor byl načten, ale běh je kratší než váš cíl.')
          : p('The file was read. This run has not been counted.', 'Soubor byl načten. Tento běh nebyl započten.');
  const explanation = accepted
    ? p('The saved decision accepts the run for this challenge. The return of your stake is a separate step.', 'Uložené rozhodnutí přijímá běh pro tuto výzvu. Vrácení vkladu je samostatný krok.')
    : review
      ? p('The distance and dates meet the goal, but the recording needs manual review. This result does not confirm cheating.', 'Vzdálenost a termín splňují cíl, ale záznam vyžaduje ruční posouzení. Tento výsledek nepotvrzuje podvádění.')
      : outside
        ? p('A LIVE challenge counts a run only when its full start-to-finish period is inside the agreed dates. A readable GPX can therefore be valid as a file and still not count here.', 'Živá výzva započítá běh, jen pokud celý jeho průběh od začátku do konce spadá do sjednaného termínu. GPX tedy může být správně načtený a přesto se sem nepočítat.')
        : shorter
          ? p('This challenge requires one activity that reaches the agreed distance. Upload another qualifying run before the upload deadline.', 'Výzva vyžaduje jednu aktivitu, která dosáhne sjednané vzdálenosti. Do uzávěrky nahrajte jiný běh, který ji splní.')
          : p('The saved decision does not accept this run. See the recorded decision and checks below; a rejected result alone does not prove cheating.', 'Uložené rozhodnutí tento běh nepřijímá. Níže najdete rozhodnutí a kontroly; samotné zamítnutí nepotvrzuje podvádění.');
  return <section className={'activity-explanation tone-' + (accepted ? 'success' : 'attention')} aria-labelledby={heading}>
    <div className="activity-explanation-title">
      {accepted ? <CheckCircle size={24} aria-hidden="true" /> : review ? <ShieldCheck size={24} aria-hidden="true" /> : <Info size={24} aria-hidden="true" />}
      <div><h4 id={heading}>{title}</h4><p>{explanation}</p></div>
    </div>
    <ol className="activity-explanation-stages" aria-label={p('File, challenge result and stake are separate steps', 'Soubor, výsledek výzvy a vklad jsou samostatné kroky')}>
      <EvidenceStage number={1} title={p('File read successfully', 'Soubor úspěšně načten')} description={p('The activity data was parsed and saved. This is not proof of who ran.', 'Data aktivity byla načtena a uložena. Tím se neověřuje, kdo běžel.')} tone="success" />
      <EvidenceStage number={2} title={accepted ? p('Run counted', 'Běh započten') : review ? p('Waiting for review', 'Čeká na posouzení') : p('Run not counted', 'Běh nezapočten')} description={accepted ? p('The saved decision accepts this activity.', 'Uložené rozhodnutí přijímá tuto aktivitu.') : review ? p('The recorded measurements need manual review.', 'Zaznamenaná měření potřebují ruční posouzení.') : outside ? p('Its dates are outside this challenge window.', 'Jeho termín nespadá do okna této výzvy.') : shorter ? p('Its distance is below this challenge goal.', 'Jeho vzdálenost nedosahuje cíle této výzvy.') : p('The recorded decision does not accept this activity.', 'Zaznamenané rozhodnutí tuto aktivitu nepřijímá.')} tone={accepted ? 'success' : 'attention'} />
      <EvidenceStage number={3} title={funds.label} description={funds.description} tone={funds.tone} />
    </ol>
    <table className="activity-explanation-comparison">
      <caption>{p('Compare this challenge with your uploaded run', 'Porovnání výzvy s nahraným během')}</caption>
      <thead><tr><th scope="col">{p('Rule', 'Pravidlo')}</th><th scope="col">{p('Your challenge', 'Vaše výzva')}</th><th scope="col">{p('Uploaded run', 'Nahraný běh')}</th></tr></thead>
      <tbody>
        <tr><th scope="row">{p('Distance', 'Vzdálenost')}</th><td data-label={p('Your challenge', 'Vaše výzva')}>{p('At least', 'Nejméně')} <strong>{number(c.target_m)}</strong></td><td data-label={p('Uploaded run', 'Nahraný běh')}><strong>{number(a.distance_m)}</strong><span className={'activity-comparison-result ' + (distanceMet ? 'result-pass' : 'result-attention')}>{distanceMet ? <CheckCircle aria-hidden="true" size={16} /> : <WarningCircle aria-hidden="true" size={16} />}{distanceMet ? p('Distance met', 'Vzdálenost splněna') : p('Too short', 'Příliš krátký')}</span></td></tr>
        <tr><th scope="row">{p('Run dates', 'Termín běhu')}</th><td data-label={p('Your challenge', 'Vaše výzva')}>{c.profile === 'REPLAY' ? <><strong>{p('Historical run allowed', 'Historický běh povolen')}</strong><span>{p('Upload deadline still applies.', 'Uzávěrka nahrání stále platí.')}</span></> : <><span>{p('From', 'Od')} <time dateTime={c.starts_at}>{displayedDate(c.starts_at, p)}</time></span><span>{p('To', 'Do')} <time dateTime={c.ends_at}>{displayedDate(c.ends_at, p)}</time></span></>}</td><td data-label={p('Uploaded run', 'Nahraný běh')}><span>{p('From', 'Od')} <time dateTime={a.starts_at}>{displayedDate(a.starts_at, p)}</time></span><span>{p('To', 'Do')} <time dateTime={a.ends_at}>{displayedDate(a.ends_at, p)}</time></span><span className={'activity-comparison-result ' + (inWindow ? 'result-pass' : 'result-attention')}>{inWindow ? <CheckCircle aria-hidden="true" size={16} /> : <WarningCircle aria-hidden="true" size={16} />}{inWindow ? p('Dates allowed', 'Termín vyhovuje') : windowKnown ? p('Outside the window', 'Mimo sjednaný termín') : p('Cannot compare dates', 'Termíny nelze porovnat')}</span></td></tr>
      </tbody>
    </table>
    <p className="activity-explanation-timezone">{p('Times above use your device’s local time zone.', 'Časy výše používají místní časové pásmo vašeho zařízení.')}</p>
    {outside && u.goal_result === 'NOT_MET' && <div className="activity-explanation-next"><FileText aria-hidden="true" size={22} /><div><strong>{p('Want to try this older run?', 'Chcete vyzkoušet tento starší běh?')}</strong><p>{p('Use a separate Historical replay challenge. A run that did not meet the current goal can be evaluated there; an activity that already met another goal cannot be counted twice. The current challenge and its stake stay as they are.', 'Použijte samostatnou výzvu Historické přehrání. Běh, který nesplnil současný cíl, v ní lze vyhodnotit; aktivitu, která už splnila jiný cíl, nelze započítat dvakrát. Současná výzva i její vklad zůstávají beze změny.')}</p>{onHistoricalReplay && <><button type="button" className="button secondary" onClick={onHistoricalReplay}>{p('Prepare a separate historical replay', 'Připravit samostatné historické přehrání')}<ArrowRight aria-hidden="true" size={18} /></button><p className="activity-explanation-button-note">{p('This opens a new draft. It does not activate a challenge or transfer any stake.', 'Otevře nový návrh. Neaktivuje výzvu ani nepřevádí žádný vklad.')}</p></>}</div></div>}
  </section>;
}

export function NextStepHint({ state, profile, network, assessment, startsAt, endsAt, uploadDeadline, refundAfter, now = Date.now(), walletLinked, signedPending, activationExpired, lastUpload }: NextStepHintProps) {
  const language = useLanguage();
  const p: PickText = (en, cs) => language === 'en' ? en : cs;
  const heading = useId();
  let title: string;
  let body: string;
  let tone: Tone = 'neutral';
  let icon: ReactNode = <ArrowRight size={23} aria-hidden="true" />;
  if (signedPending) {
    title = p('Next: check the transaction already sent.', 'Další krok: ověřit už odeslanou transakci.');
    body = p('Use Check latest status to check its confirmation. Do not sign another transfer while this transaction is pending. A successful web request alone does not confirm a token transfer.', 'Zkontrolovat aktuální stav ověří její potvrzení. Dokud transakce čeká, nepodepisujte další převod. Samotný úspěšný webový požadavek nepotvrzuje převod tokenů.');
    icon = <Clock size={23} aria-hidden="true" />;
    tone = 'attention';
  } else if (['REFUNDED', 'CANCELLED', 'EXPIRED', 'FORFEITED'].includes(state)) {
    const funds = fundsDescription(state, network, p);
    title = p('This challenge is finished.', 'Tato výzva je ukončená.');
    body = funds.description + ' ' + p('You can inspect its recorded result and history. Creating another challenge starts a separate agreement.', 'Můžete si prohlédnout zaznamenaný výsledek a historii. Nová výzva je samostatná dohoda.');
    icon = state === 'FORFEITED' ? <Info size={23} aria-hidden="true" /> : <CheckCircle size={23} aria-hidden="true" />;
    tone = funds.tone;
  } else if (state === 'DRAFT') {
    if (activationExpired) {
      title = p('Next: create a new draft with fresh dates.', 'Další krok: vytvořit nový návrh s novým termínem.');
      body = p('This draft can no longer be activated. If you already signed a transaction, refresh its status before creating anything new. Otherwise start a separate new challenge.', 'Tento návrh už nelze aktivovat. Pokud jste už podepsali transakci, před vytvořením další výzvy obnovte její stav. Jinak založte novou samostatnou výzvu.');
      tone = 'attention';
    } else if (network === 'DEVNET' && !walletLinked) {
      title = p('Next: link Phantom in My account.', 'Další krok: propojit Phantom v Účtu.');
      body = p('Sign the wallet ownership message first. Linking the wallet transfers no tokens. Then return here to lock the agreed test stake and activate the challenge.', 'Nejdříve podepište zprávu o vlastnictví peněženky. Propojení nepřevádí tokeny. Pak se vraťte sem, uzamkněte sjednaný testovací vklad a aktivujte výzvu.');
      icon = <LockKey size={23} aria-hidden="true" />;
    } else {
      title = p('Next: lock the agreed stake and activate.', 'Další krok: uzamknout sjednaný vklad a aktivovat.');
      body = network === 'LOCAL'
        ? p('Activate & lock stake opens a confirmation. Confirming reserves simulation credits for this challenge; it sends no blockchain transaction. Upload your run after activation.', 'Aktivovat a uzamknout vklad otevře potvrzení. Potvrzení rezervuje simulační kredity pro tuto výzvu; neodesílá blockchainovou transakci. Běh nahrajte až po aktivaci.')
        : p('Activate & lock stake opens a confirmation, then asks Phantom to sign a test-token deposit. Wait for the confirmed ACTIVE status before uploading your run.', 'Aktivovat a uzamknout vklad otevře potvrzení a poté požádá Phantom o podpis vkladu testovacích tokenů. Před nahráním běhu vyčkejte na potvrzený stav Aktivní.');
      icon = <LockKey size={23} aria-hidden="true" />;
    }
  } else if (state !== 'ACTIVE') {
    title = p('Next: refresh the recorded challenge status.', 'Další krok: obnovit zaznamenaný stav výzvy.');
    body = p('The current state does not identify an available next action. Check latest status reads the saved state and any pending transaction.', 'Aktuální stav neurčuje dostupný další krok. Zkontrolovat aktuální stav načte uložený stav a případnou čekající transakci.');
    tone = 'attention';
  } else if (now >= Date.parse(refundAfter)) {
    title = p('Next: request the timeout return.', 'Další krok: požádat o vrácení po timeoutu.');
    body = p('The timeout return is now available. Return unsettled stake opens a confirmation for returning the stake to its owner. The stake is released only when settlement is confirmed.', 'Vrácení po timeoutu je nyní dostupné. Vrátit nevypořádaný vklad otevře potvrzení vrácení vkladu jeho vlastníkovi. Vklad se uvolní až po potvrzeném vypořádání.');
    icon = <Clock size={23} aria-hidden="true" />;
  } else if (assessment === 'MET') {
    title = now < Date.parse(startsAt)
      ? p('Your run counts. The return is available after the challenge starts.', 'Běh se počítá. Vrácení bude dostupné po začátku výzvy.')
      : p('Next: return the test stake to its owner.', 'Další krok: vrátit testovací vklad jeho vlastníkovi.');
    body = now < Date.parse(startsAt)
      ? p('The goal is recorded as met. The return cannot be requested before', 'Cíl je zaznamenaný jako splněný. O vrácení nelze požádat před') + ' ' + displayedDate(startsAt, p) + '.'
      : network === 'LOCAL'
        ? p('Return test stake opens a confirmation for releasing the reserved simulation credits to their owner. The run result and the credit return are separate steps.', 'Vrátit testovací vklad otevře potvrzení uvolnění rezervovaných simulačních kreditů jejich vlastníkovi. Výsledek běhu a vrácení kreditů jsou samostatné kroky.')
        : p('Return test stake opens a confirmation for returning test tokens to the challenge owner. The backend signs this settlement; Phantom does not need to sign a second deposit. Wait for Returned and its confirmed transaction.', 'Vrátit testovací vklad otevře potvrzení vrácení testovacích tokenů vlastníkovi výzvy. Toto vypořádání podepisuje backend; Phantom nepodepisuje druhý vklad. Vyčkejte na stav Vráceno a potvrzenou transakci.');
    if (lastUpload?.decision === 'REJECTED') body = p('An earlier accepted run still meets the goal, even though the latest upload did not count.', 'Dřívější přijatý běh stále splňuje cíl, i když se poslední nahraná aktivita nezapočítala.') + ' ' + body;
    icon = <CheckCircle size={23} aria-hidden="true" />;
    tone = 'success';
  } else if (assessment === 'REVIEW_REQUIRED') {
    title = p('Next: follow the manual review.', 'Další krok: sledovat ruční posouzení.');
    body = p('The recording needs an operator’s decision. It is not a confirmed fraud result. The stake stays locked; Check latest status reads any recorded review outcome.', 'Záznam potřebuje rozhodnutí operátora. Nejde o potvrzený podvod. Vklad zůstává uzamčený; Zkontrolovat aktuální stav načte případný zaznamenaný výsledek posouzení.');
    icon = <ShieldCheck size={23} aria-hidden="true" />;
    tone = 'attention';
  } else if (now > Date.parse(uploadDeadline)) {
    title = p('The upload deadline has passed.', 'Uzávěrka nahrání uplynula.');
    body = p('You can no longer upload another activity to this challenge. The operator can resolve a missed goal; a timeout return becomes available at', 'Do této výzvy už nelze nahrát další aktivitu. Operátor může vypořádat nesplněný cíl; vrácení po timeoutu bude dostupné od') + ' ' + displayedDate(refundAfter, p) + '. ' + p('Check latest status checks the recorded outcome.', 'Zkontrolovat aktuální stav ověří zaznamenaný výsledek.');
    icon = <Clock size={23} aria-hidden="true" />;
    tone = 'attention';
  } else if (profile === 'LIVE' && now < Date.parse(startsAt)) {
    title = p('Next: complete a run during the agreed dates.', 'Další krok: uběhnout běh ve sjednaném termínu.');
    body = p('Your LIVE challenge starts at', 'Vaše živá výzva začíná') + ' ' + displayedDate(startsAt, p) + '. ' + p('An older run will not count. Cancel & return stake opens a confirmation to close this challenge and return its stake to the owner; it does not edit its dates.', 'Starší běh se nezapočítá. Zrušit a vrátit vklad otevře potvrzení uzavření výzvy a vrácení vkladu vlastníkovi; neupravuje její termín.');
    icon = <Clock size={23} aria-hidden="true" />;
  } else {
    title = lastUpload?.decision === 'REJECTED'
      ? p('Next: upload another run that meets this challenge.', 'Další krok: nahrát jiný běh, který splní tuto výzvu.')
      : p('Next: upload your GPX or FIT activity.', 'Další krok: nahrát svou aktivitu GPX nebo FIT.');
    body = profile === 'REPLAY'
      ? p('Historical replay accepts a previously completed activity that has not met another challenge’s goal. Upload file & check goal reads the file and evaluates this goal; it does not return the stake.', 'Historické přehrání přijímá dříve dokončenou aktivitu, která ještě nesplnila cíl jiné výzvy. Nahrát soubor a ověřit cíl načte soubor a vyhodnotí tento cíl; nevrací vklad.')
      : p('The full run must take place between', 'Celý běh musí proběhnout mezi') + ' ' + displayedDate(startsAt, p) + ' ' + p('and', 'a') + ' ' + displayedDate(endsAt, p) + '. ' + p('Upload file & check goal reads the file and evaluates this goal; it does not return the stake.', 'Nahrát soubor a ověřit cíl načte soubor a vyhodnotí tento cíl; nevrací vklad.');
    icon = <FileText size={23} aria-hidden="true" />;
  }
  return <section className={'challenge-next-step tone-' + tone} aria-labelledby={heading}>
    {icon}<div><span className="challenge-next-step-label">{p('WHAT TO DO NOW', 'CO UDĚLAT TEĎ')}</span><h4 id={heading}>{title}</h4><p>{body}</p></div>
  </section>;
}
