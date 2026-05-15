export interface DeployRecord {
  commit_hash: string;
  short_hash: string;
  deployed_at: string;
  cache_path: string | null;
  log_path: string | null;
  success: boolean;
  build_duration_secs: number | null;
}

export interface HealthStatus {
  ok: boolean;
  last_check: string;
}

export interface TargetSummary {
  name: string;
  label: string;
  repo: string;
  branch: string;
  deployed: DeployRecord | null;
  local_commit: string | null;
  remote_commit: string | null;
  process_running: boolean;
  health_url: string | null;
  group: string | null;
  service_type: string;
  health_ok: boolean | null;
}

export interface TargetListResponse {
  targets: TargetSummary[];
}

export interface StatusResponse {
  name: string;
  label: string;
  repo: string;
  branch: string;
  deployed: DeployRecord | null;
  local_commit: string | null;
  remote_commit: string | null;
  interval_secs: number;
  process_running: boolean;
  health_url: string | null;
  build_cmd: string;
  run_cmd: string | null;
  run_mode: string;
  jvm_args: string | null;
  envs: Record<string, string>;
  auto_deploy_paused: boolean;
  group: string | null;
  service_type: string;
  pid: number | null;
  uptime_secs: number | null;
  health_status: HealthStatus | null;
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

export interface BranchesResponse {
  branches: string[];
  current: string;
}

export async function fetchBranches(name: string): Promise<BranchesResponse> {
  const res = await fetch(`${API}/targets/${encodeURIComponent(name)}/branches`);
  return res.json();
}

export async function fetchTarget(name: string): Promise<{ status: string; remote_head: string }> {
  const res = await fetch(`${API}/targets/${encodeURIComponent(name)}/fetch`, { method: 'POST' });
  return res.json();
}

export async function cloneTarget(name: string, newName: string, repo?: string): Promise<{ status: string; name: string }> {
  const res = await fetch(`${API}/targets/${encodeURIComponent(name)}/clone`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ new_name: newName, repo: repo || undefined }),
  });
  return res.json();
}

export async function fetchVersion(): Promise<{ version: string }> {
  const res = await fetch(`${API}/version`);
  return res.json();
}

export function restartServer(): void {
  fetch(`${API}/restart`, { method: 'POST' });
}

export async function fetchQueue(): Promise<QueueResponse> {
  const res = await fetch(`${API}/queue`);
  return res.json();
}

export interface ConfigResponse {
  target: string;
  path: string;
  content: string;
}

export async function fetchConfig(name: string): Promise<ConfigResponse> {
  const res = await fetch(`${API}/targets/${encodeURIComponent(name)}/config`);
  return res.json();
}

export async function saveConfig(name: string, content: string): Promise<void> {
  await fetch(`${API}/targets/${encodeURIComponent(name)}/config`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ content }),
  });
}

export interface MavenSettingsResponse {
  target: string;
  path: string;
  content: string;
}

export async function fetchMavenSettings(name: string): Promise<MavenSettingsResponse> {
  const res = await fetch(`${API}/targets/${encodeURIComponent(name)}/maven-settings`);
  return res.json();
}

export async function saveMavenSettings(name: string, content: string): Promise<void> {
  await fetch(`${API}/targets/${encodeURIComponent(name)}/maven-settings`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ content }),
  });
}

export interface LocalRepoResponse {
  target: string;
  local_repo: string;
}

export async function fetchLocalRepo(name: string): Promise<LocalRepoResponse> {
  const res = await fetch(`${API}/targets/${encodeURIComponent(name)}/local-repo`);
  return res.json();
}

export interface ViteConfigResponse {
  target: string;
  path: string;
  content: string;
}

export async function fetchViteConfig(name: string): Promise<ViteConfigResponse> {
  const res = await fetch(`${API}/targets/${encodeURIComponent(name)}/vite-config`);
  return res.json();
}

export async function saveViteConfig(name: string, content: string): Promise<void> {
  await fetch(`${API}/targets/${encodeURIComponent(name)}/vite-config`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ content }),
  });
}

export interface EnvResponse {
  target: string;
  jvm_args: string | null;
  envs: Record<string, string>;
}

export async function fetchEnv(name: string): Promise<EnvResponse> {
  const res = await fetch(`${API}/targets/${encodeURIComponent(name)}/env`);
  return res.json();
}

export async function stopTarget(name: string): Promise<{ status: string }> {
  const res = await fetch(`${API}/targets/${encodeURIComponent(name)}/stop`, { method: 'POST' });
  return res.json();
}

export async function autoDeployToggle(name: string): Promise<{ status: string; auto_deploy_paused: boolean }> {
  const res = await fetch(`${API}/targets/${encodeURIComponent(name)}/auto-deploy`, { method: 'POST' });
  return res.json();
}

export async function saveEnv(name: string, jvm_args: string | null, envs: Record<string, string>): Promise<void> {
  await fetch(`${API}/targets/${encodeURIComponent(name)}/env`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ jvm_args, envs }),
  });
}

export async function deleteTarget(name: string, keepFiles?: boolean): Promise<{ status: string }> {
  const res = await fetch(`${API}/targets/${encodeURIComponent(name)}`, {
    method: 'DELETE',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ keep_files: keepFiles ?? false }),
  });
  if (!res.ok) throw new Error((await res.json() as any).message || 'Delete failed');
  return res.json();
}

export async function renameTarget(name: string, newName: string): Promise<{ status: string }> {
  const res = await fetch(`${API}/targets/${encodeURIComponent(name)}/rename`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ new_name: newName }),
  });
  if (!res.ok) throw new Error((await res.json() as any).message || 'Rename failed');
  return res.json();
}

export interface CreateTargetParams {
  name: string;
  label?: string;
  repo: string;
  branch?: string;
  group?: string;
  profile?: string;
  git_remote?: string;
  build_cmd?: string;
  artifact?: string;
  run_cmd?: string;
  health_url?: string;
  run_mode?: string;
  jvm_args?: string;
  maven_settings?: string;
  local_repo?: string;
  envs?: Record<string, string>;
}

export async function createTarget(params: CreateTargetParams): Promise<{ status: string }> {
  const res = await fetch(`${API}/targets`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(params),
  });
  if (!res.ok) throw new Error((await res.json() as any).message || 'Create failed');
  return res.json();
}
