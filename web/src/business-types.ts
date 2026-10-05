export type Organization = { id: string; name: string; owner_id: string; role: 'OWNER' | 'ADMIN' | 'MEMBER' | 'OPERATOR'; version: number; archived_at?: string | null };
export type TeamMember = { user_id: string; display_name: string; email?: string; role: 'OWNER' | 'ADMIN' | 'MEMBER'; joined_at?: string };
export type Invitation = { id: string; email: string; role: 'ADMIN' | 'MEMBER'; status: string; expires_at: string };
export type ProgramTemplate = 'LEGACY' | 'ACTIVITY_POINTS' | 'EMPLOYER_MATCH' | 'EVENT' | 'MONTHLY_BUDGET';
export type CompanyProgram = {
  id: string; organization_id?: string; title: string; target_m: number; currency: 'CZK'; reward_minor: number;
  max_participants: number; state: 'DRAFT' | 'PUBLISHED' | 'CLOSED' | 'ARCHIVED'; version: number;
  reward_units?: number | null; budget_units?: number | null; reserved_units?: number | null; paid_units?: number | null;
  funder_id?: string | null; profile?: 'LIVE' | 'REPLAY' | null; starts_at?: string | null; ends_at?: string | null;
  upload_deadline?: string | null; review_deadline?: string | null; published_at?: string | null; closed_at?: string | null;
  template?: ProgramTemplate; point_reward?: number; point_stake?: number; monthly_decline?: boolean;
  returned_units?: number; previous_cycle_id?: string | null; next_cycle_id?: string | null;
  cycle_starts_at?: string | null; cycle_ends_at?: string | null;
};
export type ProgramInput = Pick<CompanyProgram, 'id' | 'title' | 'target_m' | 'max_participants'> & {
  currency?: 'CZK'; reward_minor?: number; template?: ProgramTemplate; point_reward?: number; point_stake?: number; monthly_decline?: boolean;
};
export type Enrollment = {
  id: string; user_id: string; display_name?: string; state: 'ENROLLED' | 'REWARDED' | 'CLOSED';
  assessment: string; reward_units: number; created_at: string; paid_at?: string | null;
  staked_points?: number; awarded_points?: number; consented_at?: string | null; provisional_points?: number;
};
export type CompanyEvent = { id: number; kind: string; created_at: string };
export type WorkspaceManagement = { can_delete: boolean; delete_reason: string | null; can_archive: boolean; archive_reason: string | null };
export type ProgramClosure = { allowed: boolean; reason: string | null; checked_at: string; available_at: string | null; unpaid_rewards: number; pending_reviews: number; participant_count: number; rewarded_count: number };
export type OrganizationDetail = {
  organization: Organization; programs: CompanyProgram[]; events: CompanyEvent[];
  members?: TeamMember[]; invitations?: Invitation[]; enrollments?: Enrollment[]; management?: WorkspaceManagement | null;
};
export type ActivityResult = {
  format: string; distance_m: number; elapsed_seconds: number; starts_at: string; ends_at: string; sample_count: number;
  source_authenticity?: string; reasons?: string[]; checks?: { code: string; outcome: string; detail: string }[];
};
export type CompanyUpload = { id: string; activity: ActivityResult; goal_result: string; decision: string; reason: string; received_at?: string; content_deleted_at?: string | null };
export type ProgramDetail = { program: CompanyProgram; participants: Enrollment[]; participant_count: number; current_user_enrollment?: Enrollment | null; is_operator?: boolean; closure?: ProgramClosure | null; server_now?: string };
export type EnrollmentDetail = { program: CompanyProgram; enrollment: Enrollment; uploads: CompanyUpload[]; events?: CompanyEvent[]; is_operator?: boolean };
export type PublishInput = { version: number; reward_units: number; profile: 'LIVE' | 'REPLAY'; starts_at: string; ends_at: string; upload_deadline: string; review_deadline: string };
export type CompanyPoints = {
  unit: 'POINTS'; simulation: true; pool_available_points?: number; pool_reserved_points?: number; total_awarded_points?: number;
  own_available_points: number; own_staked_points: number; movements?: { id: string; kind: string; created_at: string; amount_points: number; employee_delta: number; stake_delta: number }[];
};
export const credits = (units: number | null | undefined, language: 'en' | 'cs') => new Intl.NumberFormat(language === 'en' ? 'en-GB' : 'cs-CZ', { maximumFractionDigits: 6 }).format((units ?? 0) / 1e6);
export const points = (value: number | null | undefined, language: 'en' | 'cs') => new Intl.NumberFormat(language === 'en' ? 'en-GB' : 'cs-CZ', { maximumFractionDigits: 0 }).format(value ?? 0);
export const templateName = (template: ProgramTemplate | undefined, language: 'en' | 'cs') => ({
  LEGACY: ['Earlier test-credit program', 'Starší program s testovacími kredity'],
  ACTIVITY_POINTS: ['Points for activity', 'Body za aktivitu'],
  EMPLOYER_MATCH: ['Employer Match', 'Spoluúčast firmy'],
  EVENT: ['Team event', 'Týmová akce'],
  MONTHLY_BUDGET: ['Monthly bonus budget', 'Měsíční bonusový rozpočet'],
}[template ?? 'LEGACY'][language === 'en' ? 0 : 1]);
export const companyPath = (org: string, program?: string) => `/api/organizations/${org}${program ? `/programs/${program}` : ''}`;
