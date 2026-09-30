export type Guide = {
  trash_id: string;
  name: string;
  url?: string;
  groups: { trash_id: string; name: string; default: boolean }[];
};
export type Target = {
  service_id: string;
  name: string;
  kind: string;
  trash_id: string;
  quality_sizes: boolean;
  reset_scores: boolean;
  groups: { add: { trash_id: string }[]; skip: string[] };
  overrides: Record<string, number | boolean>;
  profile_name: string | null;
  error: string | null;
};
