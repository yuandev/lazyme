// Performance monitoring is integrated directly in apiFetch

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
  memory_kb: number | null;
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
  memory_kb: number | null;
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

const CACHE_TTL = 30000;
const statusCache = new Map<string, { data: StatusResponse; time: number }>();
const commitsCache = new Map<string, { data: CommitInfo[]; time: number }>();
const historyCache = new Map<string, { data: DeployRecord[]; time: number }>();
const branchesCache = new Map<string, { data: BranchesResponse; time: number }>();

function getCached<T>(cache: Map<string, { data: T; time: number }>, key: string): T | null {
  const entry = cache.get(key);
  if (entry && Date.now() - entry.time < CACHE_TTL) return entry.data;
  return null;
}

export function clearCache(name?: string) {
  if (name) {
    statusCache.delete(name);
    commitsCache.delete(name);
    historyCache.delete(name);
    branchesCache.delete(name);
  } else {
    statusCache.clear();
    commitsCache.clear();
    historyCache.clear();
    branchesCache.clear();
  }
}

function token(): string | null {
  return sessionStorage.getItem('lazyme_token');
}

function authHeaders(): Record<string, string> {
  const t = token();
  return t ? { 'Authorization': `Bearer ${t}` } : {};
}

export function wsToken(): string {
  const t = token();
  return t ? `?token=${encodeURIComponent(t)}` : '';
}

export function getToken(): string | null { return token(); }
export function setToken(t: string) { sessionStorage.setItem('lazyme_token', t); }
export function clearToken() { sessionStorage.removeItem('lazyme_token'); }

async function apiFetch(path: string, init?: RequestInit): Promise<Response> {
  const start = performance.now();
  const method = init?.method || 'GET';
  try {
    const res = await fetch(`${API}${path}`, {
      ...init,
      headers: { ...authHeaders(), ...(init?.headers || {}) },
    });
    if (res.status === 401) { clearToken(); window.location.reload(); }
    const duration = performance.now() - start;
    const statKey = `${method} ${path.split('/').slice(0, 3).join('/')}`;
    const existing = apiMetrics.get(statKey) || { count: 0, totalTime: 0, slowCount: 0, errorCount: 0 };
    existing.count++;
    existing.totalTime += duration;
    if (duration > 1000) existing.slowCount++;
    if (!res.ok) existing.errorCount++;
    apiMetrics.set(statKey, existing);
    return res;
  } catch (err) {
    const duration = performance.now() - start;
    const statKey = `${method} ${path.split('/').slice(0, 3).join('/')}`;
    const existing = apiMetrics.get(statKey) || { count: 0, totalTime: 0, slowCount: 0, errorCount: 0 };
    existing.count++;
    existing.totalTime += duration;
    existing.errorCount++;
    apiMetrics.set(statKey, existing);
    throw err;
  }
}

export const apiMetrics = new Map<string, { count: number; totalTime: number; slowCount: number; errorCount: number }>();

export function getApiPerformanceSummary() {
  const result: Record<string, any> = {};
  apiMetrics.forEach((stat, name) => {
    result[name] = {
      count: stat.count,
      avgTime: Math.round(stat.totalTime / stat.count),
      slowRate: `${Math.round(stat.slowCount / stat.count * 100)}%`,
      errorRate: `${Math.round(stat.errorCount / stat.count * 100)}%`,
    };
  });
  return result;
}

function apiPath(name: string): string { return `/targets/${encodeURIComponent(name)}`; }
const hdr = () => ({ 'Content-Type': 'application/json' });

export async function fetchTargets(): Promise<TargetSummary[]> {
  const res = await apiFetch('/targets');
  const data: TargetListResponse = await res.json();
  return data.targets;
}

export async function fetchTargetStatus(name: string): Promise<StatusResponse> {
  const cached = getCached(statusCache, name);
  if (cached) return cached;
  const res = await apiFetch(`${apiPath(name)}/status`);
  const data = await res.json();
  statusCache.set(name, { data, time: Date.now() });
  return data;
}

export async function fetchTargetCommits(name: string): Promise<CommitInfo[]> {
  const cached = getCached(commitsCache, name);
  if (cached) return cached;
  const res = await apiFetch(`${apiPath(name)}/commits`);
  const data: CommitsResponse = await res.json();
  commitsCache.set(name, { data: data.commits, time: Date.now() });
  return data.commits;
}

export async function fetchTargetHistory(name: string): Promise<DeployRecord[]> {
  const cached = getCached(historyCache, name);
  if (cached) return cached;
  const res = await apiFetch(`${apiPath(name)}/history`);
  const data: HistoryResponse = await res.json();
  historyCache.set(name, { data: data.history, time: Date.now() });
  return data.history;
}

export async function fetchTargetLogs(name: string, hash: string): Promise<string> {
  const res = await apiFetch(`${apiPath(name)}/logs/${hash}`);
  if (!res.ok) return '';
  const data = await res.json();
  return data.content;
}

export async function deployTarget(name: string): Promise<void> {
  await apiFetch(`${apiPath(name)}/deploy`, { method: 'POST' });
  clearCache(name);
}

export async function rollbackTarget(name: string, commit: string): Promise<void> {
  await apiFetch(`${apiPath(name)}/rollback`, { method: 'POST', headers: hdr(), body: JSON.stringify({ commit }) });
  clearCache(name);
}

export async function switchBranch(name: string, branch: string): Promise<void> {
  await apiFetch(`${apiPath(name)}/branch`, { method: 'POST', headers: hdr(), body: JSON.stringify({ branch }) });
  clearCache(name);
}

export interface BranchesResponse { branches: string[]; current: string; }

export async function fetchBranches(name: string): Promise<BranchesResponse> {
  const cached = getCached(branchesCache, name);
  if (cached) return cached;
  const res = await apiFetch(`${apiPath(name)}/branches`);
  const data = await res.json();
  branchesCache.set(name, { data, time: Date.now() });
  return data;
}

export async function fetchTarget(name: string): Promise<{ status: string; remote_head: string }> {
  const res = await apiFetch(`${apiPath(name)}/fetch`, { method: 'POST' });
  const data = await res.json();
  clearCache(name);
  return data;
}

export async function cloneTarget(name: string, newName: string, repo?: string): Promise<{ status: string; name: string }> {
  const res = await apiFetch(`${apiPath(name)}/clone`, { method: 'POST', headers: hdr(), body: JSON.stringify({ new_name: newName, repo: repo || undefined }) });
  return res.json();
}

export async function fetchVersion(): Promise<{ version: string }> {
  const res = await apiFetch('/version');
  return res.json();
}

export function restartServer(): void {
  apiFetch('/restart', { method: 'POST' });
}

export async function fetchQueue(): Promise<QueueResponse> {
  const res = await apiFetch('/queue');
  return res.json();
}

export interface ConfigResponse { target: string; path: string; content: string; }

export async function fetchConfig(name: string): Promise<ConfigResponse> {
  const res = await apiFetch(`${apiPath(name)}/config`);
  return res.json();
}

export async function saveConfig(name: string, content: string): Promise<void> {
  await apiFetch(`${apiPath(name)}/config`, { method: 'PUT', headers: hdr(), body: JSON.stringify({ content }) });
}

export interface MavenSettingsResponse { target: string; path: string; content: string; }

export async function fetchMavenSettings(name: string): Promise<MavenSettingsResponse> {
  const res = await apiFetch(`${apiPath(name)}/maven-settings`);
  return res.json();
}

export async function saveMavenSettings(name: string, content: string): Promise<void> {
  await apiFetch(`${apiPath(name)}/maven-settings`, { method: 'PUT', headers: hdr(), body: JSON.stringify({ content }) });
}

export interface LocalRepoResponse { target: string; local_repo: string; }

export async function fetchLocalRepo(name: string): Promise<LocalRepoResponse> {
  const res = await apiFetch(`${apiPath(name)}/local-repo`);
  return res.json();
}

export interface ViteConfigResponse { target: string; path: string; content: string; }

export async function fetchViteConfig(name: string): Promise<ViteConfigResponse> {
  const res = await apiFetch(`${apiPath(name)}/vite-config`);
  return res.json();
}

export async function saveViteConfig(name: string, content: string): Promise<void> {
  await apiFetch(`${apiPath(name)}/vite-config`, { method: 'PUT', headers: hdr(), body: JSON.stringify({ content }) });
}

export interface EnvResponse { target: string; jvm_args: string | null; envs: Record<string, string>; }

export async function fetchEnv(name: string): Promise<EnvResponse> {
  const res = await apiFetch(`${apiPath(name)}/env`);
  return res.json();
}

export async function stopTarget(name: string): Promise<{ status: string }> {
  const res = await apiFetch(`${apiPath(name)}/stop`, { method: 'POST' });
  clearCache(name);
  return res.json();
}

export async function autoDeployToggle(name: string): Promise<{ status: string; auto_deploy_paused: boolean }> {
  const res = await apiFetch(`${apiPath(name)}/auto-deploy`, { method: 'POST' });
  clearCache(name);
  return res.json();
}

export async function saveEnv(name: string, jvm_args: string | null, envs: Record<string, string>): Promise<void> {
  await apiFetch(`${apiPath(name)}/env`, { method: 'PUT', headers: hdr(), body: JSON.stringify({ jvm_args, envs }) });
}

export async function deleteTarget(name: string, keepFiles?: boolean): Promise<{ status: string }> {
  const res = await apiFetch(`${apiPath(name)}`, { method: 'DELETE', headers: hdr(), body: JSON.stringify({ keep_files: keepFiles ?? false }) });
  if (!res.ok) throw new Error((await res.json() as any).message || 'Delete failed');
  return res.json();
}

export async function renameTarget(name: string, newName: string): Promise<{ status: string }> {
  const res = await apiFetch(`${apiPath(name)}/rename`, { method: 'POST', headers: hdr(), body: JSON.stringify({ new_name: newName }) });
  if (!res.ok) throw new Error((await res.json() as any).message || 'Rename failed');
  return res.json();
}

export interface CreateTargetParams {
  name: string; label?: string; repo: string; branch?: string; group?: string; profile?: string;
  git_remote?: string; build_cmd?: string; artifact?: string; run_cmd?: string;
  health_url?: string; run_mode?: string; jvm_args?: string; maven_settings?: string;
  local_repo?: string; envs?: Record<string, string>;
}

export async function createTarget(params: CreateTargetParams): Promise<{ status: string }> {
  const res = await apiFetch('/targets', { method: 'POST', headers: hdr(), body: JSON.stringify(params) });
  if (!res.ok) throw new Error((await res.json() as any).message || 'Create failed');
  return res.json();
}
