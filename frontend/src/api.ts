export interface DeployRecord {
  commit_hash: string;
  short_hash: string;
  deployed_at: string;
  cache_path: string | null;
  success: boolean;
}

export interface StatusResponse {
  deployed: DeployRecord | null;
  local_commit: string | null;
  remote_commit: string | null;
  branch: string;
  polling: boolean;
  interval_secs: number;
}

export interface CommitInfo {
  hash: string;
  short_hash: string;
  message: string;
  author: string;
  timestamp: string;
}

const API = '/api';

export async function fetchStatus(): Promise<StatusResponse> {
  const res = await fetch(`${API}/status`);
  return res.json();
}

export async function fetchCommits(): Promise<CommitInfo[]> {
  const res = await fetch(`${API}/commits`);
  const data = await res.json();
  return data.commits;
}

export async function fetchHistory(): Promise<DeployRecord[]> {
  const res = await fetch(`${API}/history`);
  const data = await res.json();
  return data.history;
}

export async function deployLatest(): Promise<void> {
  await fetch(`${API}/deploy`, { method: 'POST' });
}

export async function rollback(commit: string): Promise<void> {
  await fetch(`${API}/rollback`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ commit }),
  });
}
