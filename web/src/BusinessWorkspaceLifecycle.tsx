import { useState, type FormEvent } from 'react';
import { Archive, ArrowCounterClockwise, ShieldCheck } from '@phosphor-icons/react';
import { api } from './api';
import { Field, PasswordField } from './components';
import { ConfirmActionDialog } from './ConfirmActionDialog';
import { useLanguage } from './i18n';
import { companyPath, type Organization, type WorkspaceManagement } from './business-types';

export function BusinessWorkspaceLifecycle({ organization, management, busy, run, onChanged, onPrograms }: {
  organization: Organization; management?: WorkspaceManagement | null; busy: boolean;
  run: (action: () => Promise<void>) => Promise<void>;
  onChanged: (deleted: boolean) => Promise<void>; onPrograms: () => void;
}) {
  const language = useLanguage(); const p = (en: string, cs: string) => language === 'en' ? en : cs;
  const [deleting, setDeleting] = useState(false);
  const [confirmation, setConfirmation] = useState<'archive' | 'restore' | null>(null);
  if (organization.role !== 'OWNER') return null;
  const archived = !!organization.archived_at;
  const reason = (code: string | null | undefined) => ({
    WORKSPACE_HAS_OTHER_MEMBERS: p('This workspace has other members. You can archive it after its programs are closed; members and history will be retained.', 'Prostor má další členy. Po uzavření programů jej můžete archivovat; členové i historie zůstanou zachovaní.'),
    BUSINESS_HISTORY_MUST_BE_RETAINED: p('A program has already used test credits. Its funding and reward history must remain available. Use Archive workspace to remove it from the active list.', 'Program už použil testovací kredity. Historie financování a odměn musí zůstat dostupná. Pro odebrání z aktivního seznamu použijte Archivovat prostor.'),
    WORKSPACE_HAS_PUBLISHED_PROGRAMS: p('Close each published program first. Open its detail to see the remaining deadline, rewards or reviews.', 'Nejprve uzavřete všechny zveřejněné programy. V detailu programu uvidíte zbývající lhůtu, odměny nebo posouzení.'),
    WORKSPACE_RESERVED_CREDITS: p('The workspace still has reserved test credits. Settle its programs before archiving.', 'Prostor stále obsahuje rezervované testovací kredity. Před archivací vypořádejte programy.'),
    WORKSPACE_ARCHIVED: p('This workspace is archived. Restore it to make changes.', 'Prostor je archivovaný. Pro změny jej nejprve obnovte.'),
  } as Record<string, string>)[code ?? ''] ?? p('Refresh workspace data to check whether this action is available.', 'Obnovte údaje prostoru pro ověření dostupnosti této akce.');
  function changeArchive() {
    const action = confirmation;
    if (!action) return;
    void run(async () => {
      try {
        await api(`${companyPath(organization.id)}/${action}`, { method: 'POST', body: { version: organization.version } });
        setConfirmation(null); setDeleting(false); await onChanged(false);
      } catch (error) { setConfirmation(null); throw error; }
    });
  }
  function remove(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); if (!management?.can_delete) return;
    const form = new FormData(event.currentTarget);
    void run(async () => {
      await api(companyPath(organization.id), { method: 'DELETE', body: { password: String(form.get('password')), confirmation: String(form.get('confirmation')) } });
      setDeleting(false); await onChanged(true);
    });
  }
  return <section className="workspace-lifecycle" aria-label={p('Workspace cleanup', 'Úklid prostoru')}>
    <div className="company-local-note"><Archive size={21} aria-hidden="true" /><div><strong>{archived ? p('Archived workspace', 'Archivovaný prostor') : p('Archive workspace', 'Archivovat prostor')}</strong><p>{archived ? p('This workspace is kept in the archive. Programs, members and recorded rewards remain available. Restore it to work with it again.', 'Prostor je uložený v archivu. Programy, členové a zaznamenané odměny zůstávají dostupné. Pro další práci jej obnovte.') : p('Remove this workspace from the active list after its programs are settled. Keep its history and restore it whenever needed.', 'Po vypořádání programů odeberte prostor z aktivního seznamu. Historie zůstane zachovaná a prostor lze kdykoliv obnovit.')}</p></div></div>
    {!archived && !management?.can_archive && <p className="body-copy" id={`archive-blocker-${organization.id}`}>{reason(management?.archive_reason)}</p>}
    <div className="form-actions"><button type="button" className="button secondary" disabled={busy || (!archived && !management?.can_archive)} aria-describedby={!archived && !management?.can_archive ? `archive-blocker-${organization.id}` : undefined} onClick={() => setConfirmation(archived ? 'restore' : 'archive')}>{archived ? <ArrowCounterClockwise size={18} aria-hidden="true" /> : <Archive size={18} aria-hidden="true" />}{archived ? p('Restore workspace', 'Obnovit prostor') : p('Archive workspace', 'Archivovat prostor')}</button>{!archived && management?.archive_reason === 'WORKSPACE_HAS_PUBLISHED_PROGRAMS' && <button type="button" className="button text-button" disabled={busy} onClick={onPrograms}>{p('View programs to close', 'Zobrazit programy k uzavření')}</button>}</div>
    {!archived && <details className="section-disclosure destructive-disclosure"><summary>{p('Permanent deletion', 'Trvalé smazání')}</summary><div className="disclosure-content"><p className="body-copy">{management?.can_delete ? p('This workspace has no other members and has never funded a program. Permanent deletion removes it and its drafts.', 'Prostor nemá další členy a nikdy nefinancoval program. Trvalé smazání odstraní prostor i jeho návrhy.') : reason(management?.delete_reason)}</p>{!management?.can_delete ? <p className="field-hint"><ShieldCheck size={16} aria-hidden="true" /> {p('Permanent deletion is unavailable. Archiving retains records without keeping the workspace in the active list.', 'Trvalé smazání není dostupné. Archivace zachová záznamy a odebere prostor z aktivního seznamu.')}</p> : !deleting ? <button type="button" className="link-button danger-link" disabled={busy} onClick={() => setDeleting(true)}>{p('Delete workspace permanently', 'Trvale smazat prostor')}</button> : <form onSubmit={remove}><fieldset disabled={busy}><Field label={p('Type the workspace name', 'Napište název prostoru')}><input name="confirmation" required autoComplete="off" /></Field><PasswordField label={p('Current password', 'Současné heslo')} /><div className="form-actions"><button className="button danger">{p('Permanently delete workspace', 'Trvale smazat prostor')}</button><button className="button text-button" type="button" onClick={() => setDeleting(false)}>{p('Cancel', 'Zrušit')}</button></div></fieldset></form>}</div></details>}
    {confirmation && <ConfirmActionDialog title={confirmation === 'archive' ? p('Archive this workspace?', 'Archivovat tento prostor?') : p('Restore this workspace?', 'Obnovit tento prostor?')} label={confirmation === 'archive' ? p('Archive workspace', 'Archivovat prostor') : p('Restore workspace', 'Obnovit prostor')} busy={busy} onClose={() => setConfirmation(null)} onConfirm={changeArchive}><p><strong>{organization.name}</strong></p><p>{confirmation === 'archive' ? p('The workspace will move into Archived workspaces for its members. No credits will be transferred, and no programs, members or private activity files will be deleted.', 'Prostor se členům přesune do Archivovaných prostorů. Nepřevedou se žádné kredity a nesmažou se programy, členové ani soukromé soubory aktivit.') : p('The workspace will return to the active list. Its programs and recorded rewards keep their current state.', 'Prostor se vrátí do aktivního seznamu. Programy a zaznamenané odměny si zachovají současný stav.')}</p></ConfirmActionDialog>}
  </section>;
}
