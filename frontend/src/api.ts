export interface DeployRecord {
  commit_hash: string;
  short_hash: string;
  deployed_at: string;
  cache_path: string | null;
  log_path: string | null;
  success: boolean;
}

export interface TargetSummary {
  name: string;
  repo: string;
  branch: string;
  deployed: DeployRecord | null;
  local_commit: string | null;
  remote_commit: string | null;
  process_running: boolean;
  health_url: string | null;
}

export interface TargetListResponse {
  targets: TargetSummary[];
}

export interface StatusResponse {
  name: string;
  repo: string;
  branch: string;
  deployed: DeployRecord | null;
  local_commit: string | null;
  remote_commit: string | null;
  interval_secs: number;
  process_running: boolean;
  health_url: string | null;
}

export interface CommitInfo {
  hash: string;
  short_hash: string;
  message: string;
  author: string;
  timestamp: string;
}

export interface CommitsResponse {
  commits: CommitInfo[];
}

export interface HistoryResponse {
  history: DeployRecord[];
}

export interface QueueResponse {
  building: string | null;
}

const API = '/api';

export async function fetchTargets(): Promise<TargetSummary[]> {
  const res = await fetch(`${API}/targets`);
  const data: TargetListResponse = await res.json();
  return data.targets;
}

export async function fetchTargetStatus(name: string): Promise<StatusResponse> {
  const res = await fetch(`${API}/targets/${encodeURIComponent(name)}/status`);
  return res.json();
}

export async function fetchTargetCommits(name: string): Promise<CommitInfo[]> {
  const res = await fetch(`${API}/targets/${encodeURIComponent(name)}/commits`);
  const data: CommitsResponse = await res.json();
  return data.commits;
}

export async function fetchTargetHistory(name: string): Promise<DeployRecord[]> {
  const res = await fetch(`${API}/targets/${encodeURIComponent(name)}/history`);
  const data: HistoryResponse = await res.json();
  return data.history;
}

export async function fetchTargetLogs(name: string, hash: string): Promise<string> {
  const res = await fetch(`${API}/targets/${encodeURIComponent(name)}/logs/${hash}`);
  if (!res.ok) return '';
  const data = await res.json();
  return data.content;
}

export async function deployTarget(name: string): Promise<void> {
  await fetch(`${API}/targets/${encodeURIComponent(name)}/deploy`, { method: 'POST' });
}

export async function rollbackTarget(name: string, commit: string): Promise<void> {
  await fetch(`${API}/targets/${encodeURIComponent(name)}/rollback`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ commit }),
  });
}

export async function switchBranch(name: string, branch: string): Promise<void> {
  await fetch(`${API}/targets/${encodeURIComponent(name)}/branch`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ branch }),
  });
}

export async function fetchQueue(): Promise<QueueResponse> {
  const res = await fetch(`${API}/queue`);
  return res.json();
}
