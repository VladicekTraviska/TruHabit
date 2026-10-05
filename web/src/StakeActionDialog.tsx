import { useEffect, useId, useRef } from 'react';
import { ArrowRight, Info, LockKey, X } from '@phosphor-icons/react';
import { useLanguage } from './i18n';
import './stake-action-dialog.css';

export type StakeAction = 'DEPOSIT' | 'SUCCESS' | 'CANCEL' | 'FAILURE' | 'TIMEOUT';
export function StakeActionDialog({ action, title, amountUnits, network, canProceed, onClose, onConfirm }: {
  action: StakeAction; title: string; amountUnits: number; network: 'LOCAL' | 'DEVNET';
  canProceed: boolean; onClose: () => void; onConfirm: () => void;
}) {
  const language = useLanguage();
  const p = (en: string, cs: string) => language === 'en' ? en : cs;
  const ref = useRef<HTMLDialogElement>(null);
  const heading = useId();
  useEffect(() => {
    const dialog = ref.current;
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    dialog?.showModal();
    return () => { dialog?.close(); if (previousFocus?.isConnected) previousFocus.focus(); };
  }, []);
  const deposit = action === 'DEPOSIT';
  const forfeiture = action === 'FAILURE';
  const walletSigned = deposit || action === 'CANCEL' || action === 'TIMEOUT';
  const amount = new Intl.NumberFormat(language === 'en' ? 'en-GB' : 'cs-CZ', { maximumFractionDigits: 6 }).format(amountUnits / 1e6);
  const unit = network === 'LOCAL' ? p('simulation credits', 'simulačních kreditů') : 'THT';
  const labels: Record<StakeAction, string> = {
    DEPOSIT: p('Activate & lock stake', 'Aktivovat a uzamknout vklad'),
    SUCCESS: p('Return test stake', 'Vrátit testovací vklad'),
    CANCEL: p('Cancel & return stake', 'Zrušit a vrátit vklad'),
    FAILURE: p('Confirm failure & forfeit stake', 'Potvrdit neúspěch a propadnutí'),
    TIMEOUT: p('Return unsettled stake', 'Vrátit nevypořádaný vklad'),
  };
  const explanation: Record<StakeAction, string> = {
    DEPOSIT: p('This activates your challenge and reserves the agreed stake. It is unavailable for other challenges until settlement. Saving a draft did not do this.', 'Tímto aktivujete výzvu a uzamknete sjednaný vklad. Do vypořádání ho nelze použít na jiné výzvy. Uložení návrhu tento krok neprovedlo.'),
    SUCCESS: p('The challenge has a qualifying accepted run. This requests the return of its stake; the recorded transfer must complete before the challenge shows Returned.', 'Výzva má započítaný vyhovující běh. Tímto požádáte o vrácení vkladu; stav Vráceno se zobrazí až po dokončení zaznamenaného převodu.'),
    CANCEL: p('This ends the challenge before its run window starts and requests the return of its stake. You will not be able to upload a run to this cancelled challenge.', 'Tímto ukončíte výzvu před začátkem období běhu a požádáte o vrácení vkladu. Do zrušené výzvy už nelze nahrát běh.'),
    FAILURE: p('This settles a challenge without an accepted result and sends the test stake to its fixed failure recipient. A completed transfer cannot be reversed by a review request.', 'Tímto vypořádáte výzvu bez přijatého výsledku a odešlete testovací vklad pevnému příjemci při neúspěchu. Dokončený převod nelze zvrátit žádostí o kontrolu.'),
    TIMEOUT: p('The recovery deadline has passed. This requests the return of the still unsettled stake. It closes the challenge without marking the running goal as completed.', 'Uplynul termín pro nouzové vrácení. Tímto požádáte o vrácení dosud nevypořádaného vkladu. Výzva se uzavře, běžecký cíl se tím neoznačí za splněný.'),
  };
  return <dialog ref={ref} className="stake-dialog" aria-labelledby={heading} onCancel={onClose} onClose={onClose}>
    <div className="stake-dialog-header"><span><LockKey size={23} aria-hidden="true" />{p('Before you confirm', 'Před potvrzením')}</span><button type="button" onClick={onClose} aria-label={p('Close stake confirmation', 'Zavřít potvrzení vkladu')}><X size={22} aria-hidden="true" /></button></div>
    <h2 id={heading}>{labels[action]}</h2><p className="stake-dialog-challenge">{title}</p>
    <div className="stake-dialog-transfer"><span>{deposit ? p('Available balance', 'Volný zůstatek') : p('Locked stake', 'Uzamčený vklad')}</span><ArrowRight size={24} aria-hidden="true" /><strong>{amount} {unit}<small>{deposit ? p('Locked for this challenge', 'Uzamčeno pro tuto výzvu') : forfeiture ? p('Fixed failure recipient', 'Pevný příjemce při neúspěchu') : p('Back to the challenge owner', 'Zpět vlastníkovi výzvy')}</small></strong></div>
    <p>{explanation[action]}</p>
    <p className="stake-dialog-environment"><Info size={20} aria-hidden="true" /><span>{network === 'LOCAL' ? p('Local simulation: changes test credits only. No blockchain transaction or real payment.', 'Lokální simulace: mění pouze testovací kredity. Bez blockchainové transakce a skutečné platby.') : walletSigned ? p('Solana Devnet: test THT only. Phantom asks you to sign this transfer and your wallet pays fees in test SOL. A signature alone does not confirm the transfer.', 'Solana Devnet: pouze testovací THT. Phantom vás požádá o podpis tohoto převodu a peněženka hradí poplatky v testovacích SOL. Samotný podpis nepotvrzuje převod.') : p('Solana Devnet: test THT only. The backend signs this settlement and pays its network fee. The result is confirmed only after the blockchain confirms the transfer.', 'Solana Devnet: pouze testovací THT. Toto vypořádání podepisuje backend a hradí síťový poplatek. Výsledek je potvrzen až po potvrzení převodu blockchainem.')}</span></p>
    {!canProceed && <p role="status">{p('This action is no longer available. Close this window and check the latest challenge status.', 'Tato akce už není dostupná. Zavřete okno a zkontrolujte aktuální stav výzvy.')}</p>}
    <div className="stake-dialog-actions"><button type="button" className="button secondary" onClick={onClose}>{p('Go back', 'Zpět')}</button><button type="button" className="button primary" disabled={!canProceed} onClick={onConfirm}>{labels[action]}</button></div>
  </dialog>;
}
