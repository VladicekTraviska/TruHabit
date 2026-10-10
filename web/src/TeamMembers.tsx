import { useState, type FormEvent } from 'react';
import { Copy, Plus, Users, XCircle } from '@phosphor-icons/react';
import { api } from './api';
import { Field } from './components';
import { useLanguage } from './i18n';
import { date } from './types';
import { ConfirmActionDialog } from './ConfirmActionDialog';
import { companyPath, type Invitation, type OrganizationDetail } from './business-types';

type MemberCommand = { user_id: string; label: string; kind: 'remove' | 'transfer' | 'role'; role?: 'ADMIN' | 'MEMBER' };
export function TeamMembers({ detail, userId, busy, run, refresh }: {
  detail: OrganizationDetail; userId: string; busy: boolean; run: (action: () => Promise<void>) => Promise<void>; refresh: () => Promise<void>;
}) {
  const language = useLanguage();
  const p = (en: string, cs: string) => language === 'en' ? en : cs;
  const org = detail.organization;
  const manage = org.role !== 'MEMBER';
  const owner = org.role === 'OWNER';
  const [inviteId, setInviteId] = useState(() => crypto.randomUUID());
  const [inviteCode, setInviteCode] = useState('');
  const [inviteEmail, setInviteEmail] = useState('');
  const [copyNotice, setCopyNotice] = useState('');
  const [command, setCommand] = useState<MemberCommand | null>(null);
  const [showInvite, setShowInvite] = useState(false);

  function invite(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    const email = String(data.get('email')).trim();
    const role = String(data.get('role'));
    void run(async () => {
      const result = await api<{ invitation: Invitation; token?: string }>(`${companyPath(org.id)}/invitations`, { method: 'POST', body: { id: inviteId, email, role } });
      setInviteCode(result.token ?? ''); setInviteEmail(email); setCopyNotice(result.token ? '' : p('The invitation was already created, but its secret code cannot be recovered. Revoke it and create a new invitation.', 'Pozvánka už byla vytvořená, ale její tajný kód nelze obnovit. Odvolejte ji a vytvořte novou.'));
      setInviteId(crypto.randomUUID()); setShowInvite(false); await refresh();
    });
  }
  async function copyCode() {
    try { await navigator.clipboard.writeText(inviteCode); setCopyNotice(p('Invitation code copied. Share it only with its recipient.', 'Kód pozvánky zkopírován. Předejte jej jen příjemci.')); }
    catch { setCopyNotice(p('Select the code below and copy it manually.', 'Označte kód níže a ručně jej zkopírujte.')); }
  }
  function perform() {
    if (!command) return;
    const selected = command;
    void run(async () => {
      if (selected.kind === 'transfer') await api(`${companyPath(org.id)}/transfer-owner`, { method: 'POST', body: { user_id: selected.user_id, version: org.version } });
      else if (selected.kind === 'remove') await api(`${companyPath(org.id)}/members/${selected.user_id}`, { method: 'DELETE' });
      else await api(`${companyPath(org.id)}/members/${selected.user_id}`, { method: 'PATCH', body: { role: selected.role } });
      setCommand(null); await refresh();
    });
  }
  return <section className="company-members" aria-labelledby="team-members-title">
    <div className="company-section-title"><div><h3 id="team-members-title"><Users size={20} aria-hidden="true" />{p('Team members', 'Členové týmu')}<span className="company-member-count">{detail.members?.length ?? 0}</span></h3><p className="field-hint">{p('Membership is explicit. A company email domain never grants access automatically.', 'Členství se přijímá výslovně. Firemní e-mailová doména sama nikdy neuděluje přístup.')}</p></div>{manage && <button className="button secondary" disabled={busy} aria-expanded={showInvite} onClick={() => setShowInvite(value => !value)}><Plus size={17} aria-hidden="true" />{p('Invite a member', 'Pozvat člena')}</button>}</div>
    {showInvite && <form className="company-program" onSubmit={invite}><fieldset disabled={busy}><Field label={p('Recipient email', 'E-mail příjemce')} hint={p('The recipient must sign in with this exact email to accept.', 'Příjemce se musí přihlásit účtem s tímto e-mailem.')}><input name="email" type="email" autoComplete="off" required maxLength={254} /></Field><Field label={p('Role', 'Role')}><select name="role"><option value="MEMBER">{p('Member — join published programs', 'Člen — účast ve zveřejněných programech')}</option>{owner && <option value="ADMIN">{p('Administrator — manage programs', 'Správce — správa programů')}</option>}</select></Field><p className="body-copy">{p('We create a private one-time code for manual sharing. No email will be sent.', 'Vytvoříme soukromý jednorázový kód k ručnímu předání. E-mail se neposílá.')}</p><div className="form-actions"><button className="button primary">{p('Create invitation', 'Vytvořit pozvánku')}</button><button type="button" className="button text-button" onClick={() => setShowInvite(false)}>{p('Cancel', 'Zrušit')}</button></div></fieldset></form>}
    {inviteCode && <div className="company-invite-code"><strong>{p('Private invitation code for', 'Soukromý kód pozvánky pro')} {inviteEmail}</strong><p className="field-hint">{p('Shown once. The recipient opens For teams → Accept invitation. Do not put this code in a public presentation.', 'Zobrazuje se jednou. Příjemce otevře Pro týmy → Přijmout pozvánku. Kód neukazujte na veřejné prezentaci.')}</p><textarea readOnly aria-label={p('Private invitation code', 'Soukromý kód pozvánky')} value={inviteCode} rows={2} /><div className="form-actions"><button className="button secondary" onClick={() => void copyCode()}><Copy size={17} aria-hidden="true" />{p('Copy invitation code', 'Kopírovat kód pozvánky')}</button><button className="button text-button" onClick={() => { setInviteCode(''); setCopyNotice(''); }}>{p('Hide code', 'Skrýt kód')}</button></div>{copyNotice && <p role="status">{copyNotice}</p>}</div>}
    <div className="company-member-list">{(detail.members ?? []).map(member => <article className="company-member-row" key={member.user_id}><div className="company-member-identity"><span className="company-member-avatar" aria-hidden="true">{member.display_name.slice(0, 2).toUpperCase()}</span><div><strong>{member.display_name}</strong><small><span className="company-role-badge">{member.role === 'OWNER' ? p('Owner', 'Vlastník') : member.role === 'ADMIN' ? p('Administrator', 'Správce') : p('Member', 'Člen')}</span>{member.user_id === userId ? p(' · you', ' · vy') : ''}</small></div></div>{owner && member.role !== 'OWNER' && <div className="company-row-actions"><button className="button text-button" disabled={busy} onClick={() => setCommand({ user_id: member.user_id, label: member.display_name, kind: 'role', role: member.role === 'ADMIN' ? 'MEMBER' : 'ADMIN' })}>{member.role === 'ADMIN' ? p('Make member', 'Nastavit jako člena') : p('Make administrator', 'Nastavit jako správce')}</button><button className="button text-button" disabled={busy} onClick={() => setCommand({ user_id: member.user_id, label: member.display_name, kind: 'transfer' })}>{p('Transfer ownership', 'Předat vlastnictví')}</button><button className="button text-button danger-link" disabled={busy} onClick={() => setCommand({ user_id: member.user_id, label: member.display_name, kind: 'remove' })}>{p('Remove member', 'Odebrat člena')}</button></div>}</article>)}</div>
    {manage && !!detail.invitations?.length && <details className="section-disclosure"><summary>{p('Invitations', 'Pozvánky')}</summary><div className="disclosure-content">{detail.invitations.map(invitation => <article className="company-member-row" key={invitation.id}><div><strong>{invitation.email}</strong><small>{invitation.status} · {p('expires', 'platí do')} {date(invitation.expires_at)}</small></div>{invitation.status === 'OPEN' && <button className="button text-button" disabled={busy} onClick={() => void run(async () => { await api(`${companyPath(org.id)}/invitations/${invitation.id}/revoke`, { method: 'POST', body: {} }); await refresh(); })}><XCircle size={17} aria-hidden="true" />{p('Revoke', 'Odvolat')}</button>}</article>)}</div></details>}
    {!inviteCode && copyNotice && <p role="status">{copyNotice}</p>}
    {!detail.members?.length && <p className="field-hint">{p('Refresh the workspace to load its members.', 'Obnovte prostor pro načtení členů.')}</p>}
    {command && <ConfirmActionDialog busy={busy} title={command.kind === 'transfer' ? p('Transfer workspace ownership?', 'Předat vlastnictví prostoru?') : command.kind === 'remove' ? p('Remove this team member?', 'Odebrat tohoto člena?') : p('Change this member’s role?', 'Změnit roli tohoto člena?')} label={p('Confirm change', 'Potvrdit změnu')} onClose={() => setCommand(null)} onConfirm={perform}><p><strong>{command.label}</strong></p><p>{command.kind === 'transfer' ? p('You will become an administrator. The new owner will control this workspace. Existing program budgets stay with their original funder.', 'Stanete se správcem. Nový vlastník bude spravovat tento prostor. Existující rozpočty programů zůstávají původnímu financujícímu účtu.') : command.kind === 'remove' ? p('Access will be revoked. The server prevents removal when active program participation still needs settlement.', 'Přístup bude odebrán. Server zabrání odebrání, pokud je nutné nejprve vypořádat aktivní účast v programu.') : command.role === 'ADMIN' ? p('Administrators can manage programs and invitations. They cannot read private running recordings.', 'Správci mohou spravovat programy a pozvánky. Nemohou číst soukromé záznamy běhu.') : p('This member will be able to join published programs but will lose administrative access.', 'Člen se bude moci účastnit zveřejněných programů, ale ztratí správcovský přístup.')}</p></ConfirmActionDialog>}
  </section>;
}
