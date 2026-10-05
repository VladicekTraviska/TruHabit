import { locale } from './i18n';

export interface User {
  id: string;
  email: string;
  display_name: string;
  email_verified_at: string | null;
  created_at: string;
}
export interface Session {
  user: User;
  csrf_token: string;
  wallet: string | null;
}
export interface Readiness {
  accounts: boolean;
  dev_reset?: boolean;
  email_delivery: boolean;
  activity_provider: boolean;
  deposits: boolean;
  reason: string;
}
export interface Goal {
  currency: 'CZK' | 'USD';
  id: string;
  user_id: string;
  target_m: number;
  pledge_cents: number;
  starts_at: string;
  ends_at: string;
  state: 'DRAFT' | 'ARCHIVED';
  policy_version: string;
  version: number;
  created_at: string;
  updated_at: string;
}
export interface GoalInput {
  currency?: 'CZK' | 'USD';
  target_m: number;
  pledge_cents: number;
  starts_at: string;
}
export interface GoalPage {
  goals: Goal[];
  next_cursor: string | null;
}
export interface GoalEvent {
  id: number;
  kind: string;
  created_at: string;
  detail: Record<string, unknown>;
}
export const money = (cents: number, currency: 'CZK' | 'USD' = 'USD') =>
  new Intl.NumberFormat(locale(), {
    style: 'currency',
    currency,
    minimumFractionDigits: 0,
    maximumFractionDigits: 2,
  }).format(cents / 100);
export const date = (iso: string) =>
  new Intl.DateTimeFormat(locale(), {
    day: 'numeric',
    month: 'short',
    year: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(iso));
