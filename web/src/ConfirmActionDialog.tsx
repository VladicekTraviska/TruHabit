import { useEffect, useId, useRef, type ReactNode } from 'react';
import { ShieldCheck, X } from '@phosphor-icons/react';
import { useLanguage } from './i18n';
import './stake-action-dialog.css';

export function ConfirmActionDialog({ title, children, label, busy, confirmDisabled = false, onClose, onConfirm }: {
  title: string; children: ReactNode; label: string; busy: boolean; confirmDisabled?: boolean; onClose: () => void; onConfirm: () => void;
}) {
  const language = useLanguage();
  const p = (en: string, cs: string) => language === 'en' ? en : cs;
  const ref = useRef<HTMLDialogElement>(null);
  const heading = useId();
  useEffect(() => {
    const dialog = ref.current;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    dialog?.showModal();
    return () => { dialog?.close(); if (previous?.isConnected) previous.focus(); };
  }, []);
  return <dialog ref={ref} className="stake-dialog" aria-labelledby={heading} onCancel={event => { event.preventDefault(); if (!busy) onClose(); }}>
    <div className="stake-dialog-header"><span><ShieldCheck size={22} aria-hidden="true" />{p('Before you confirm', 'Před potvrzením')}</span><button type="button" disabled={busy} onClick={onClose} aria-label={p('Close confirmation', 'Zavřít potvrzení')}><X size={22} aria-hidden="true" /></button></div>
    <h2 id={heading}>{title}</h2><div className="company-confirm-content">{children}</div>
    <div className="stake-dialog-actions"><button className="button secondary" type="button" disabled={busy} onClick={onClose}>{p('Go back', 'Zpět')}</button><button className="button primary" type="button" disabled={busy || confirmDisabled} onClick={onConfirm}>{busy ? p('Saving…', 'Ukládám…') : label}</button></div>
  </dialog>;
}
