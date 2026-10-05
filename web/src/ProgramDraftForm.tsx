import { useEffect, useRef, useState, type FormEvent } from 'react';
import { ArrowRight, Check } from '@phosphor-icons/react';
import { Field } from './components';
import { useLanguage } from './i18n';
import { money } from './types';
import type { CompanyProgram, ProgramInput } from './business-types';

export function ProgramDraftForm({ initial, busy, onSave, onCancel }: {
  initial: CompanyProgram | null; busy: boolean; onSave: (input: ProgramInput) => Promise<void>; onCancel: () => void;
}) {
  const language = useLanguage(); const p = (en: string, cs: string) => language === 'en' ? en : cs;
  const [id] = useState(() => initial?.id ?? crypto.randomUUID());
  const [reward, setReward] = useState(initial ? initial.reward_minor / 100 : 400);
  const [participants, setParticipants] = useState(initial?.max_participants ?? 5);
  const [review, setReview] = useState<ProgramInput | null>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  useEffect(() => { if (review) heading.current?.focus(); }, [review]);
  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); const fields = new FormData(event.currentTarget);
    setReview({ id, title: String(fields.get('title')).trim(), target_m: Number(fields.get('target_m')), currency: 'CZK', reward_minor: Math.round(reward * 100), max_participants: participants });
  }
  return <div className="company-program program-form"><ol className="form-stepper" aria-label={p('Program setup', 'Nastavení programu')}><li className={review ? 'complete' : 'current'}><span>{review ? <Check size={14} aria-hidden="true" /> : '1'}</span>{p('Draft', 'Návrh')}</li><li className={review ? 'current' : ''}><span>2</span>{p('Review', 'Kontrola')}</li></ol>
    <form onSubmit={submit} hidden={!!review}><h3>{initial ? p('Edit program draft', 'Upravit návrh programu') : p('New program', 'Nový program')}</h3><fieldset disabled={busy}>
      <Field label={p('Program name', 'Název programu')}><input name="title" defaultValue={initial?.title ?? ''} required minLength={2} maxLength={100} /></Field>
      <Field label={p('Goal distance', 'Cílová vzdálenost')}><select name="target_m" defaultValue={initial?.target_m ?? 3000}>{[1000, 3000, 5000].map(value => <option key={value} value={value}>{value / 1000} km</option>)}</select></Field>
      <div className="company-form-grid"><Field label={p('Maximum participants', 'Maximum účastníků')}><input type="number" required min={1} max={10000} step={1} value={participants} onChange={event => setParticipants(Number(event.target.value))} /></Field><Field label={p('Planning estimate per person (CZK)', 'Plánovaný odhad na osobu (Kč)')}><input type="number" required min={1} max={10000} step={1} value={reward} onChange={event => setReward(Number(event.target.value))} /></Field></div>
      <p className="program-budget"><span>{p('CZK planning estimate', 'Plánovaný odhad v Kč')}</span><strong>{money(reward * participants * 100, 'CZK')}</strong></p>
      <p className="field-hint">{p('This preserves your planning amount. In the next step, choose a separate reward in LOCAL test credits and fund the program. No conversion or real payment is made.', 'Tímto se uloží plánovaná částka. V dalším kroku zvolíte samostatnou odměnu v LOCAL testovacích kreditech a program financujete. Neproběhne přepočet ani skutečná platba.')}</p>
      <div className="form-actions"><button className="button primary">{p('Review draft', 'Zkontrolovat návrh')}<ArrowRight size={17} aria-hidden="true" /></button><button className="button text-button" type="button" onClick={onCancel}>{p('Cancel', 'Zrušit')}</button></div>
    </fieldset></form>
    {review && <><h3 tabIndex={-1} ref={heading}>{p('Review program draft', 'Zkontrolujte návrh programu')}</h3><h4>{review.title}</h4><dl className="company-agreement"><dt>{p('Distance', 'Vzdálenost')}</dt><dd>{review.target_m / 1000} km</dd><dt>{p('Maximum participants', 'Maximum účastníků')}</dt><dd>{review.max_participants}</dd><dt>{p('Planned estimate per person', 'Plánovaný odhad na osobu')}</dt><dd>{money(review.reward_minor, 'CZK')}</dd><dt>{p('Total CZK estimate', 'Celkový odhad v Kč')}</dt><dd>{money(review.reward_minor * review.max_participants, 'CZK')}</dd></dl><p className="body-copy">{p('Saving does not reserve funds or start enrollment. Open the draft afterward to fund and publish the test program.', 'Uložení nerezervuje prostředky ani nespouští přihlašování. Potom otevřete návrh pro financování a zveřejnění testovacího programu.')}</p><div className="form-actions"><button className="button primary" disabled={busy} onClick={() => void onSave(review)}>{p('Save program draft', 'Uložit návrh programu')}</button><button className="button secondary" disabled={busy} onClick={() => setReview(null)}>{p('Back to settings', 'Zpět k nastavení')}</button></div></>}
  </div>;
}
