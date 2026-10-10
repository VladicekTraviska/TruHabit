import { useEffect, useRef, useState, type FormEvent } from 'react';
import { ArrowDownLeft, Coins, LockSimple, Plus, ShieldCheck, Trophy, Wallet } from '@phosphor-icons/react';
import { api } from './api';
import { Field } from './components';
import { ConfirmActionDialog } from './ConfirmActionDialog';
import { companyPath, points, type CompanyPoints, type Organization } from './business-types';
import { date } from './types';
import { useLanguage } from './i18n';

export function BusinessPoints({ organization, revision, busy, run, onChanged, onError }: {
  organization: Organization; revision: number; busy: boolean;
  run: (action: () => Promise<void>) => Promise<void>; onChanged: () => Promise<void>; onError: (error: unknown) => void;
}) {
  const language = useLanguage(); const p = (en: string, cs: string) => language === 'en' ? en : cs;
  const [balance, setBalance] = useState<CompanyPoints | null>(null);
  const [loading, setLoading] = useState(true);
  const [adding, setAdding] = useState(false);
  const [amount, setAmount] = useState('5000');
  const [confirming, setConfirming] = useState(false);
  const [requestId, setRequestId] = useState(() => crypto.randomUUID());
  const generation = useRef(0);
  const owner = organization.role === 'OWNER';
  const manager = owner || organization.role === 'ADMIN';
  useEffect(() => {
    const at = ++generation.current; setLoading(true);
    void api<CompanyPoints>(`${companyPath(organization.id)}/points`).then(result => { if (generation.current === at) setBalance(result); }).catch(error => { if (generation.current === at) onError(error); }).finally(() => { if (generation.current === at) setLoading(false); });
    return () => { generation.current++; };
  }, [organization.id, revision]);
  function prepare(event: FormEvent<HTMLFormElement>) { event.preventDefault(); setConfirming(true); }
  function topUp() {
    void run(async () => {
      await api(`${companyPath(organization.id)}/points/top-up`, { method: 'POST', body: { id: requestId, points: Number(amount) } });
      setRequestId(crypto.randomUUID()); setConfirming(false); setAdding(false); await onChanged();
    });
  }
  const show = (value: number | undefined) => value === undefined || !balance ? '—' : points(value, language);
  const movementNames: Record<string, string> = {
    REWARD: p('Earned program award', 'Získaná odměna programu'),
    STAKE_LOCK: p('Voluntary pledge locked', 'Dobrovolná garance uzamčena'),
    STAKE_REFUND: p('Pledge returned after success', 'Garance vrácena po úspěchu'),
    STAKE_FORFEIT: p('Unmet pledge settled to company pool', 'Nesplněná garance vypořádána do firemního poolu'),
  };
  return <section className="company-points-dashboard" aria-busy={loading} aria-label={p('Company points', 'Firemní body')}>
    <div className="company-section-title"><div><h3><Wallet size={21} aria-hidden="true" />{manager ? p('Company pool & your points', 'Firemní pool a vaše body') : p('Your benefit points', 'Vaše benefitní body')}</h3><p className="field-hint">{p('Points are stored in this company workspace. No Phantom wallet is needed.', 'Body jsou uložené v tomto firemním prostoru. Phantom peněženku nepotřebujete.')}</p></div>{owner && !organization.archived_at && <button className="button secondary" disabled={busy || loading} onClick={() => setAdding(value => !value)}><Plus size={17} aria-hidden="true" />{p('Add demo points to pool', 'Doplnit demo body do poolu')}</button>}</div>
    {manager && <div className="company-points-metrics manager">
      <div className="company-points-main"><div className="company-pool-label"><Wallet size={20} aria-hidden="true" /><span>{p('Available company pool', 'Volný firemní pool')}</span></div><strong>{show(balance?.pool_available_points)} <small>{p('points', 'bodů')}</small></strong><p>{p('Ready to fund new programs', 'Připraveno pro nové programy')}</p></div>
      <div><LockSimple size={22} aria-hidden="true" /><span>{p('Reserved for programs', 'Rezervováno pro programy')}</span><strong>{show(balance?.pool_reserved_points)} <small>{p('points', 'bodů')}</small></strong><p>{p('Company-funded rewards', 'Odměny hrazené firmou')}</p></div>
      <div><Trophy size={22} aria-hidden="true" /><span>{p('Rewards actually paid', 'Skutečně vyplacené odměny')}</span><strong>{show(balance?.total_awarded_points)} <small>{p('points', 'bodů')}</small></strong><p>{p('Result-based benefits', 'Benefity za výsledek')}</p></div>
    </div>}
    <div className="company-personal-points"><div><Coins size={22} aria-hidden="true" /><span><span>{p('Your earned points', 'Vaše získané body')}</span><strong>{show(balance?.own_available_points)} <small>{p('points', 'bodů')}</small></strong><small>{p('Available for voluntary Employer Match', 'Dostupné pro dobrovolnou spoluúčast')}</small></span></div><div><LockSimple size={22} aria-hidden="true" /><span><span>{p('Your locked deposits', 'Vaše uzamčené vklady')}</span><strong>{show(balance?.own_staked_points)} <small>{p('points', 'bodů')}</small></strong><small>{p('Only programs you opted into', 'Jen programy s vaším souhlasem')}</small></span></div></div>
    <p className="company-points-scope"><ShieldCheck size={16} aria-hidden="true" />{p('Functional points prototype. No real money, payroll deduction, Cafeteria transfer or B2B blockchain transaction.', 'Funkční bodový prototyp. Bez skutečných peněz, srážky ze mzdy, převodu do Cafeterie nebo B2B blockchainové transakce.')}</p>
    {!!balance?.movements?.length && <details className="section-disclosure company-point-history"><summary>{p('Your point history', 'Vaše historie bodů')} · {balance.movements.length}</summary><ul>{balance.movements.map(item => <li key={item.id}><span>{movementNames[item.kind] ?? item.kind}<time>{date(item.created_at)}</time></span><strong>{item.employee_delta > 0 ? '+' : ''}{points(item.kind === 'STAKE_FORFEIT' ? item.stake_delta : item.employee_delta, language)} {p('points', 'bodů')}</strong></li>)}</ul></details>}
    {adding && owner && !organization.archived_at && <form className="company-pool-topup" onSubmit={prepare}><Field label={p('Demo points to add', 'Počet demo bodů k doplnění')} hint={p('This issues simulated company points; it does not charge any account.', 'Vytvoří simulované firemní body; žádnému účtu se nic neúčtuje.')}><input type="number" required min={1} max={10000000} step={1} value={amount} disabled={busy} onChange={event => { setAmount(event.target.value); setRequestId(crypto.randomUUID()); }} /></Field><div className="form-actions"><button className="button primary" disabled={busy}><ArrowDownLeft size={17} aria-hidden="true" />{p('Review pool top-up', 'Zkontrolovat doplnění poolu')}</button><button className="button text-button" type="button" disabled={busy} onClick={() => setAdding(false)}>{p('Cancel', 'Zrušit')}</button></div></form>}
    {confirming && <ConfirmActionDialog title={p('Add simulated company points?', 'Doplnit simulované firemní body?')} label={p('Add demo points', 'Doplnit demo body')} busy={busy} onClose={() => setConfirming(false)} onConfirm={topUp}><p><strong>{points(Number(amount), language)} {p('points', 'bodů')}</strong> → {organization.name}</p><p>{p('The points will be available for funding programs. This is a demo issuance, not a payment or a conversion from CZK.', 'Body budou dostupné pro financování programů. Jde o demo doplnění, nikoli platbu nebo přepočet z Kč.')}</p></ConfirmActionDialog>}
  </section>;
}
