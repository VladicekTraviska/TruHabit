import { useEffect, useId, useRef, useState, type FormEvent } from 'react';
import { ArrowRight, CalendarBlank, Check, Handshake, Target, Trophy, WarningCircle } from '@phosphor-icons/react';
import { Field } from './components';
import { useLanguage } from './i18n';
import { money } from './types';
import { points, templateName, type CompanyProgram, type ProgramInput, type ProgramTemplate } from './business-types';

export function ProgramDraftForm({ initial, busy, onSave, onCancel }: {
  initial: CompanyProgram | null; busy: boolean; onSave: (input: ProgramInput) => Promise<void>; onCancel: () => void;
}) {
  const language = useLanguage(); const p = (en: string, cs: string) => language === 'en' ? en : cs;
  const [id] = useState(() => initial?.id ?? crypto.randomUUID());
  const [template, setTemplate] = useState<ProgramTemplate>(initial?.template ?? (initial ? 'LEGACY' : 'ACTIVITY_POINTS'));
  const [reward, setReward] = useState(initial?.point_reward || 100);
  const [stake, setStake] = useState(initial?.point_stake || 200);
  const [planning, setPlanning] = useState(initial ? initial.reward_minor / 100 : 400);
  const [participants, setParticipants] = useState(initial?.max_participants ?? 5);
  const [review, setReview] = useState<ProgramInput | null>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  useEffect(() => { if (review) heading.current?.focus(); }, [review]);
  const legacy = template === 'LEGACY';
  const budgetId = useId();
  const budgetLimit = legacy ? 1_000_000 : 10_000_000;
  const budgetUnit = legacy ? planning : reward;
  const totalBudget = budgetUnit * participants;
  const participantLimit = Math.min(10_000, Math.max(1, Math.floor(budgetLimit / Math.max(1, budgetUnit))));
  const budgetExceeded = totalBudget > budgetLimit;
  const templates = [
    { id: 'ACTIVITY_POINTS', icon: Target, title: p('Points for activity', 'Body za aktivitu'), label: p('Start here', 'Doporučený začátek'), text: p('A qualifying run earns company-funded points. No employee deposit.', 'Vyhovující běh přinese body hrazené firmou. Zaměstnanec nic nevkládá.'), reward: 100 },
    { id: 'EVENT', icon: Trophy, title: p('Team event', 'Týmová akce'), label: p('Voluntary participation', 'Dobrovolná účast'), text: p('Set a shared running event and its dates. Every qualifying participant earns the same reward.', 'Nastavte společnou běžeckou akci a termín. Každý vyhovující účastník získá stejnou odměnu.'), reward: 150 },
    { id: 'EMPLOYER_MATCH', icon: Handshake, title: p('Employer Match', 'Spoluúčast firmy'), label: p('An optional commitment', 'Volitelný závazek'), text: p('An employee locks their own earned points. Success returns the deposit and adds the company bonus.', 'Zaměstnanec uzamkne své získané body. Úspěch vrátí vklad a přidá firemní bonus.'), reward: 800 },
    { id: 'MONTHLY_BUDGET', icon: CalendarBlank, title: p('Monthly bonus budget', 'Měsíční bonusový rozpočet'), label: p('Optional extra bonus', 'Volitelný bonus navíc'), text: p('Offer a monthly bonus that declines over time. Points already earned are always preserved.', 'Nabídněte měsíční bonus, který postupně klesá. Už získané body vždy zůstanou.'), reward: 300 },
  ] as const;
  function selectTemplate(next: typeof templates[number]) { setTemplate(next.id); setReward(next.reward); }
  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); const fields = new FormData(event.currentTarget);
    const name = event.currentTarget.elements.namedItem('title') as HTMLInputElement;
    if (String(fields.get('title')).trim().length < 2) {
      name.setCustomValidity(p('Enter a name with at least two characters, excluding spaces.', 'Zadejte název s alespoň dvěma znaky bez okolních mezer.'));
      name.reportValidity();
      return;
    }
    if (budgetExceeded) { event.currentTarget.reportValidity(); return; }
    setReview({ id, title: String(fields.get('title')).trim(), target_m: Number(fields.get('target_m')), currency: 'CZK', reward_minor: legacy ? Math.round(planning * 100) : 100, max_participants: participants, template, point_reward: legacy ? 0 : reward, point_stake: template === 'EMPLOYER_MATCH' ? stake : 0 });
  }
  return <div className="company-program program-form">
    <ol className="form-stepper" aria-label={p('Program setup', 'Nastavení programu')}><li className={review ? 'complete' : 'current'}><span>{review ? <Check size={14} aria-hidden="true" /> : '1'}</span>{p('Choose & configure', 'Výběr a nastavení')}</li><li className={review ? 'current' : ''}><span>2</span>{p('Review', 'Kontrola')}</li></ol>
    <form onSubmit={submit} hidden={!!review}><h3>{initial ? p('Edit program draft', 'Upravit návrh programu') : p('Choose how your team earns points', 'Vyberte, jak váš tým získá body')}</h3><fieldset disabled={busy}>
      <div className="company-template-menu" role="group" aria-label={p('Motivation template', 'Motivační šablona')}>
        {templates.map(item => <button key={item.id} type="button" className={`company-template-option ${template === item.id ? 'selected' : ''}`} aria-pressed={template === item.id} onClick={() => selectTemplate(item)}><span className="company-template-top"><item.icon size={23} aria-hidden="true" /><span>{item.label}</span>{template === item.id && <Check size={18} aria-hidden="true" />}</span><strong>{item.title}</strong><span className="company-template-copy">{item.text}</span></button>)}
      </div>
      {legacy && <p className="company-local-note">{p('This is an earlier test-credit draft. Its existing funding rules stay in place unless you deliberately choose a points template above.', 'Jde o starší návrh s testovacími kredity. Původní financování zůstane zachované, pokud výše záměrně nevyberete bodovou šablonu.')}</p>}
      <Field label={p('Program name', 'Název programu')}><input name="title" defaultValue={initial?.title ?? ''} required minLength={2} maxLength={100} autoComplete="off" onInput={event => event.currentTarget.setCustomValidity('')} placeholder={p('For example, October team run', 'Například říjnový běh týmu')} /></Field>
      <div className="company-form-grid"><Field label={p('Distance in one qualifying run', 'Vzdálenost v jednom vyhovujícím běhu')} hint={p('Manual GPX/FIT running records are supported.', 'Podporovány jsou ručně nahrané GPX/FIT záznamy běhu.')}><select name="target_m" defaultValue={initial?.target_m ?? 3000}>{[1000, 3000, 5000].map(value => <option key={value} value={value}>{value / 1000} km</option>)}</select></Field><Field label={p('Maximum participants', 'Maximum účastníků')} hint={p(`Up to ${points(participantLimit, language)} at this reward.`, `Při této odměně nejvýše ${points(participantLimit, language)}.`)}><input name="max_participants" type="number" required min={1} max={participantLimit} step={1} aria-invalid={budgetExceeded || undefined} aria-describedby={budgetId} value={participants} onChange={event => setParticipants(Number(event.target.value))} /></Field></div>
      {legacy ? <Field label={p('Planning estimate per person (CZK)', 'Plánovaný odhad na osobu (Kč)')}><input type="number" required min={1} max={10000} step={1} value={planning} onChange={event => setPlanning(Number(event.target.value))} /></Field> : <div className="company-form-grid"><Field label={template === 'MONTHLY_BUDGET' ? p('Maximum monthly bonus (points)', 'Maximální měsíční bonus (body)') : p('Company reward per participant (points)', 'Firemní odměna účastníka (body)')}><input type="number" required min={1} max={100000} step={1} value={reward} onChange={event => setReward(Number(event.target.value))} /></Field>{template === 'EMPLOYER_MATCH' && <Field label={p('Employee deposit (own points)', 'Vklad zaměstnance (vlastní body)')} hint={p('Only previously earned company points can be used. Participation requires explicit consent.', 'Lze použít pouze už získané firemní body. Účast vyžaduje výslovný souhlas.')}><input type="number" required min={1} max={100000} step={1} value={stake} onChange={event => setStake(Number(event.target.value))} /></Field>}</div>}
      {template === 'EMPLOYER_MATCH' && <p className="company-template-policy">{p('Success returns the employee deposit and pays the company bonus. If the agreed goal is not met when the program is settled, the deposit goes to the company pool. A rejected upload alone does not settle the deposit.', 'Úspěch vrátí vklad zaměstnance a vyplatí firemní bonus. Pokud sjednaný cíl není při vypořádání programu splněný, vklad připadne firemnímu poolu. Samotné zamítnutí souboru vklad nevypořádá.')}</p>}
      {template === 'MONTHLY_BUDGET' && <p className="company-template-policy">{p('Only an unearned extra bonus declines; salary and earned points are untouched. The qualifying activity time fixes the reward. A following calendar month is created as a new draft, then reviewed, funded and published by the owner.', 'Klesá pouze nezískaný bonus navíc; mzda a získané body zůstávají. Čas vyhovující aktivity zafixuje odměnu. Další kalendářní měsíc vznikne jako nový návrh, který vlastník zkontroluje, financuje a zveřejní.')}</p>}
      <p className="program-budget"><span>{legacy ? p('CZK planning estimate', 'Plánovaný odhad v Kč') : p('Company pool needed at publication', 'Potřeba z firemního poolu při zveřejnění')}</span><strong>{legacy ? money(planning * participants * 100, 'CZK') : `${points(reward * participants, language)} ${p('points', 'bodů')}`}</strong></p>
      <div id={budgetId} className={`company-budget-check${budgetExceeded ? ' over-limit' : ''}`} role="status"><div>{budgetExceeded ? <WarningCircle size={20} aria-hidden="true" /> : <Check size={20} aria-hidden="true" />}<p><strong>{budgetExceeded ? p('Reduce the reward or number of places', 'Snižte odměnu nebo počet míst') : p('Within the program limit', 'V limitu programu')}</strong><span>{p('Maximum per program: ', 'Maximum na program: ')}{legacy ? money(budgetLimit * 100, 'CZK') : `${points(budgetLimit, language)} ${p('points', 'bodů')}`}. {p('Available funding is checked when you publish.', 'Dostupné financování se ověří při zveřejnění.')}</span></p></div><meter min={0} max={budgetLimit} value={Math.min(Math.max(0, totalBudget), budgetLimit)} aria-label={p('Program budget against the allowed limit', 'Rozpočet programu vůči povolenému limitu')} /></div>
      <p className="field-hint">{p('Saving creates a draft. Funding and dates are reviewed in the next step. All team programs are voluntary; private activity files are hidden from HR.', 'Uložení vytvoří návrh. Financování a termíny zkontrolujete v dalším kroku. Všechny týmové programy jsou dobrovolné; soukromé soubory aktivity jsou skryté před HR.')}</p>
      <div className="form-actions"><button className="button primary">{p('Review draft', 'Zkontrolovat návrh')}<ArrowRight size={17} aria-hidden="true" /></button><button className="button text-button" type="button" onClick={onCancel}>{p('Cancel', 'Zrušit')}</button></div>
    </fieldset></form>
    {review && <><h3 tabIndex={-1} ref={heading}>{p('Review program draft', 'Zkontrolujte návrh programu')}</h3><h4>{review.title}</h4><dl className="company-agreement"><dt>{p('Template', 'Šablona')}</dt><dd>{templateName(review.template, language)}</dd><dt>{p('Distance', 'Vzdálenost')}</dt><dd>{review.target_m / 1000} km</dd><dt>{p('Maximum participants', 'Maximum účastníků')}</dt><dd>{review.max_participants}</dd><dt>{legacy ? p('Planned estimate per person', 'Plánovaný odhad na osobu') : p('Company bonus per person', 'Firemní bonus na osobu')}</dt><dd>{legacy ? money(review.reward_minor ?? 100, 'CZK') : `${points(review.point_reward, language)} ${p('points', 'bodů')}`}</dd>{review.template === 'EMPLOYER_MATCH' && <><dt>{p('Employee deposit', 'Vklad zaměstnance')}</dt><dd>{points(review.point_stake, language)} {p('own points', 'vlastních bodů')}</dd></>}<dt>{legacy ? p('Total CZK estimate', 'Celkový odhad v Kč') : p('Maximum company funding', 'Maximální financování firmou')}</dt><dd>{legacy ? money((review.reward_minor ?? 100) * review.max_participants, 'CZK') : `${points((review.point_reward ?? 0) * review.max_participants, language)} ${p('points', 'bodů')}`}</dd>{review.template === 'MONTHLY_BUDGET' && <><dt>{p('Bonus over time', 'Bonus v čase')}</dt><dd>{p('Declines only until earned', 'Klesá pouze do získání')}</dd></>}</dl><p className="body-copy">{p('Saving does not reserve funds or start enrollment. Open the saved draft to fund and publish it.', 'Uložení nerezervuje prostředky ani nespouští přihlašování. Otevřete uložený návrh pro financování a zveřejnění.')}</p><div className="form-actions"><button className="button primary" disabled={busy} onClick={() => void onSave(review)}>{p('Save program draft', 'Uložit návrh programu')}</button><button className="button secondary" disabled={busy} onClick={() => setReview(null)}>{p('Back to settings', 'Zpět k nastavení')}</button></div></>}
  </div>;
}
