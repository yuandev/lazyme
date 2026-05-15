import { useState, useEffect, useCallback, useRef } from 'react';
import {
  fetchTargets, fetchTargetStatus, fetchTargetCommits, fetchTargetHistory,
  fetchTargetLogs, deployTarget, rollbackTarget, switchBranch, fetchBranches,
  fetchTarget as fetchTargetApi, cloneTarget, fetchVersion, fetchQueue,
  fetchConfig, saveConfig, fetchMavenSettings, saveMavenSettings, fetchLocalRepo,
  fetchViteConfig, saveViteConfig, fetchEnv, saveEnv,
  restartServer, autoDeployToggle,
} from './api';
import type { TargetSummary, StatusResponse, CommitInfo, DeployRecord } from './api';
import { I18nProvider, useI18n, tf } from './i18n';

const REFRESH_MS = 8_000;

type UpdatePhase = null | 'checking' | 'downloading' | 'complete' | 'error';
interface UpdateState {
  phase: UpdatePhase;
  version: string | null;
  progress: number;
  error: string | null;
}

function AppInner() {
  const { t, lang, setLang } = useI18n();
  const [targets, setTargets] = useState<TargetSummary[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [building, setBuilding] = useState<string | null>(null);
  const [currentVersion, setCurrentVersion] = useState<string>('...');
  const [update, setUpdate] = useState<UpdateState>({ phase: null, version: null, progress: 0, error: null });
  const [liveLog, setLiveLog] = useState<{ target: string; commit: string } | null>(null);
  const [liveLines, setLiveLines] = useState<string[]>([]);
  const logEndRef = useRef<HTMLDivElement | null>(null);

  const refreshTargets = useCallback(async () => {
    const list = await fetchTargets();
    setTargets(list);
    const q = await fetchQueue();
    setBuilding(q.building);
  }, []);

  useEffect(() => {
    fetchVersion().then(v => setCurrentVersion(v.version));
    refreshTargets();
    const timer = setInterval(refreshTargets, REFRESH_MS);
    return () => clearInterval(timer);
  }, [refreshTargets]);

  useEffect(() => {
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    const ws = new WebSocket(`${proto}://${location.host}/ws`);
    ws.onmessage = (ev) => {
      try {
        const data = JSON.parse(ev.data);
        switch (data.event) {
          case 'self_update_checking': setUpdate({ phase: 'checking', version: null, progress: 0, error: null }); break;
          case 'self_update_pulling': setUpdate(prev => ({ ...prev, phase: 'downloading', progress: 0 })); break;
          case 'self_update_progress': setUpdate(prev => ({ ...prev, phase: 'downloading', progress: parseInt(data.message) || 0 })); break;
          case 'self_update_complete': setUpdate({ phase: 'complete', version: data.commit, progress: 100, error: null }); break;
          case 'self_update_error': setUpdate(prev => ({ ...prev, phase: 'error', error: data.message })); break;
          case 'build_log_start': setLiveLog({ target: data.target, commit: data.commit }); setLiveLines([]); break;
          case 'build_output': setLiveLines(prev => [...prev, data.message || '']); break;
          case 'build_log_end': setLiveLog(null); refreshTargets(); break;
          case 'build_started': setLiveLog({ target: data.target, commit: data.commit || '' }); setLiveLines([]); break;
          case 'build_complete':
          case 'targets_changed':
          case 'auto_deploy_toggled': refreshTargets(); break;
          default: if (!data.event?.startsWith('self_update_')) refreshTargets();
        }
      } catch { /* ignore */ }
    };
    ws.onclose = () => {};
    return () => { ws.close(); };
  }, [refreshTargets]);

  useEffect(() => { logEndRef.current?.scrollIntoView({ behavior: 'smooth' }); }, [liveLines]);

  const triggerSelfUpdate = async () => {
    setUpdate({ phase: 'checking', version: null, progress: 0, error: null });
    const res = await fetch('/api/self-update', { method: 'POST' });
    const data = await res.json();
    if (data.status === 'up_to_date') setUpdate({ phase: null, version: null, progress: 0, error: 'Already up to date' });
    else if (data.status === 'error') setUpdate(prev => ({ ...prev, phase: 'error', error: data.message }));
  };

  const toggleLang = () => setLang(lang === 'en' ? 'zh' : 'en');

  const isOnline = (t: TargetSummary) => t.process_running && t.health_ok !== false;

  return (
    <div style={S.container}>
      <header style={S.header}>
        <div style={S.headerLeft}>
          <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="#60a5fa" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M21 12a9 9 0 1 1-6.219-8.56"/><polyline points="21 3 21 9 15 9"/><path d="M14.5 8.5 21 3"/></svg>
          <span style={S.logoText}>lazyme</span>
          <span style={S.versionChip}>v{currentVersion}</span>
        </div>
        <div style={{ flex: 1 }} />
        {building && <span style={S.buildingBadge}><span style={S.pulse} />{t.building} {building}</span>}
        {update.version && !update.phase && (
          <span style={S.updateHint}>{tf(t.versionAvailable, { version: update.version })}</span>
        )}
        <button onClick={toggleLang} style={S.langBtn}>{lang === 'en' ? '中' : 'EN'}</button>
        {update.phase === 'downloading' ? (
          <div style={S.progressWrap}><div style={{ ...S.progressBar, width: `${update.progress}%` }} /><span style={S.progressText}>{update.progress}%</span></div>
        ) : update.phase === 'complete' ? (
          <button onClick={restartServer} style={{ ...S.updateBtn, background: '#166534', borderColor: '#22c55e', color: '#86efac' }}>{t.restartNow}</button>
        ) : (
          <button onClick={triggerSelfUpdate} disabled={!!update.phase}
            style={{ ...S.updateBtn, ...(update.phase === 'error' ? { background: '#450a0a', borderColor: '#dc2626', color: '#fca5a5' } : {}), ...(update.error === 'Already up to date' ? { borderColor: '#22c55e', color: '#86efac' } : {}) }}>
            {update.phase === 'checking' ? t.checking : update.phase === 'error' ? t.error : update.version ? tf(t.updateTo, { version: update.version }) : update.error ? t.upToDate : t.update}
          </button>
        )}
      </header>
      <div style={S.body}>
        <aside style={S.sidebar}>
          {(() => {
            const groups = new Map<string | null, TargetSummary[]>();
            for (const tg of targets) {
              const g = tg.group || null;
              if (!groups.has(g)) groups.set(g, []);
              groups.get(g)!.push(tg);
            }
            return [...groups.entries()].sort(([a], [b]) => (a || '~').localeCompare(b || '~')).map(([group, items]) => (
              <div key={group || '__u__'} style={{ marginBottom: 8 }}>
                <div style={S.groupLabel}>{group || t.dash}</div>
                {items.map(tg => (
                  <div key={tg.name} style={S.targetRow}>
                    <button onClick={() => setSelected(tg.name)} style={{ ...S.targetCard, ...(selected === tg.name ? S.targetCardActive : {}) }}>
                      <div style={S.targetTop}>
                        <span style={S.targetName}>{tg.label || tg.name}</span>
                        <span style={isOnline(tg) ? S.badgeOn : S.badgeOff}>{isOnline(tg) ? '● ' + t.online : '○ ' + t.offline}</span>
                      </div>
                      <div style={S.targetMeta}>
                        <span style={S.typeTag}>{tg.service_type}</span>
                        <span style={S.targetBranch}>{tg.branch}</span>
                        <span style={S.targetHash}>{tg.deployed?.short_hash ?? t.never}</span>
                      </div>
                    </button>
                    <button onClick={async () => {
                      const nn = prompt(t.cloneName, `${tg.name}-clone`);
                      if (!nn) return;
                      try { await cloneTarget(tg.name, nn); refreshTargets(); }
                      catch (e: any) { alert(`${t.cloneFailed} ${e.message || e}`); }
                    }} style={S.cloneBtn} title={t.clone}>
                      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>
                    </button>
                  </div>
                ))}
              </div>
            ));
          })()}
        </aside>
        <main style={S.main}>
          {liveLog && (
            <div style={S.card}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 8 }}>
                <span style={{ fontWeight: 600, fontSize: 14, color: '#93c5fd' }}>{liveLog.target}</span>
                <code style={{ fontSize: 12, color: '#6b7280', background: '#111118', padding: '1px 6px', borderRadius: 4 }}>{liveLog.commit}</code>
                <div style={{ flex: 1 }} />
                <button onClick={() => setLiveLog(null)} style={{ ...S.btnSm, background: '#1f1f2a', color: '#9ca3af' }}>✕</button>
              </div>
              <pre style={S.liveLog}><code>{liveLines.join('\n')}<div ref={logEndRef} /></code></pre>
            </div>
          )}
          {selected ? <TargetDetail name={selected} /> : <NoSelection />}
        </main>
      </div>
    </div>
  );
}

function NoSelection() {
  const { t } = useI18n();
  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', height: '100%', gap: 16 }}>
      <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="#374151" strokeWidth="1.5"><rect x="2" y="2" width="20" height="8" rx="2"/><rect x="2" y="14" width="20" height="8" rx="2"/><circle cx="8" cy="6" r="1" fill="#374151"/><circle cx="12" cy="6" r="1" fill="#374151"/><circle cx="16" cy="18" r="1" fill="#374151"/></svg>
      <span style={{ color: '#4b5563', fontSize: 14 }}>{t.selectTarget}</span>
    </div>
  );
}

function TargetDetail({ name }: { name: string }) {
  const { t } = useI18n();
  const [tab, setTab] = useState<'status' | 'commits' | 'history' | 'config'>('status');
  const [status, setStatus] = useState<StatusResponse | null>(null);
  const [commits, setCommits] = useState<CommitInfo[]>([]);
  const [history, setHistory] = useState<DeployRecord[]>([]);
  const [log, setLog] = useState('');
  const [logHash, setLogHash] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [branches, setBranches] = useState<string[]>([]);
  const [branchSel, setBranchSel] = useState('');
  const [switchingBranch, setSwitchingBranch] = useState(false);
  const [fetching, setFetching] = useState(false);

  const refresh = useCallback(async () => {
    const [s, c, h] = await Promise.all([fetchTargetStatus(name), fetchTargetCommits(name), fetchTargetHistory(name)]);
    setStatus(s); setCommits(c); setHistory(h);
    if (s.branch && !branchSel) setBranchSel(s.branch);
  }, [name]);

  useEffect(() => { refresh(); const timer = setInterval(refresh, REFRESH_MS); return () => clearInterval(timer); }, [refresh]);
  useEffect(() => { fetchBranches(name).then(b => { setBranches(b.branches); setBranchSel(b.current); }); }, [name]);

  const viewLog = async (hash: string) => { setLogHash(hash); setLog(await fetchTargetLogs(name, hash) || t.noLog); };
  const handleRollback = async (commit: string) => { setLoading(true); await rollbackTarget(name, commit); await refresh(); setLoading(false); };
  const isOnline = status && status.process_running && status.health_status?.ok !== false;
  const tabs = [
    { key: 'status' as const, label: t.status, icon: <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><rect x="3" y="3" width="18" height="18" rx="2"/><line x1="3" y1="9" x2="21" y2="9"/><line x1="9" y1="21" x2="9" y2="9"/></svg> },
    { key: 'commits' as const, label: t.commits, icon: <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><circle cx="12" cy="5" r="2"/><path d="M12 7v5"/><circle cx="12" cy="12" r="2"/><path d="M12 14v5"/><circle cx="12" cy="19" r="2"/></svg> },
    { key: 'history' as const, label: t.history, icon: <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></svg> },
    { key: 'config' as const, label: t.config, icon: <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><circle cx="12" cy="12" r="3"/><path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42"/></svg> },
  ];

  if (!status) return <div style={{ textAlign: 'center', padding: 48, color: '#6b7280' }}>{t.loading}</div>;

  return (
    <div>
      <div style={S.detailHead}>
        <div style={{ flex: 1 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 4 }}>
            <h2 style={{ fontSize: 20, fontWeight: 700, margin: 0, color: '#f1f5f9' }}>{status.label || status.name}</h2>
            <span style={isOnline ? S.badgeOn : S.badgeOff}>{isOnline ? '● ' + t.online : '○ ' + t.offline}</span>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6, flexWrap: 'wrap' }}>
            <code style={S.repoTag}>{status.repo}</code>
            <span style={S.branchTag}>@{status.branch}</span>
            {status.health_url && (
              <>
                <button onClick={() => { try { window.open(new URL(status.health_url!).origin, '_blank'); } catch { /* */ } }} style={S.openBtn} title={t.openTitle}>
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>
                  &nbsp;{t.open}
                </button>
              </>
            )}
          </div>
        </div>
      </div>

      <div style={S.tabBar}>
        {tabs.map(tb => (
          <button key={tb.key} onClick={() => setTab(tb.key)} style={{ ...S.tab, ...(tab === tb.key ? S.tabActive : {}) }}>
            {tb.icon}<span>{tb.label}</span>
          </button>
        ))}
      </div>

      {tab === 'status' && (
        <div>
          <div style={{ display: 'flex', gap: 8, marginBottom: 16 }}>
            <button onClick={async () => { setLoading(true); await deployTarget(name); await refresh(); setLoading(false); }} disabled={loading} style={{ ...S.primaryBtn, opacity: loading ? 0.5 : 1 }}>
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><polygon points="5 3 19 12 5 21 5 3"/></svg>
              {loading ? t.deploying : t.deployLatest}
            </button>
            <button onClick={async () => { await autoDeployToggle(name); await refresh(); }} style={{ ...S.primaryBtn, background: status.auto_deploy_paused ? '#064e3b' : '#451a03', color: status.auto_deploy_paused ? '#6ee7b7' : '#fbbf24', border: `1px solid ${status.auto_deploy_paused ? '#065f46' : '#78350f'}` }}>
              {status.auto_deploy_paused ? '▶ ' + t.resume : '⏸ ' + t.pause}
            </button>
            {status.auto_deploy_paused && <span style={{ fontSize: 13, color: '#f59e0b', alignSelf: 'center' }}>{t.autoDeployPaused}</span>}
          </div>
          <div style={{ ...S.card, marginBottom: 12, display: 'flex', alignItems: 'center', gap: 8 }}>
            <span style={{ fontSize: 13, color: '#9ca3af', flexShrink: 0 }}>{t.branch}</span>
            <select value={branchSel} onChange={e => setBranchSel(e.target.value)} style={S.select}>
              {branches.length === 0 && <option value={status.branch}>{status.branch}</option>}
              {branches.map(b => <option key={b} value={b}>{b}</option>)}
            </select>
            <button onClick={async () => { setSwitchingBranch(true); await switchBranch(name, branchSel); await refresh(); setSwitchingBranch(false); }} disabled={switchingBranch || !branchSel || branchSel === status.branch} style={{ ...S.smallBtn, opacity: branchSel === status.branch ? 0.4 : 1 }}>
              {switchingBranch ? '...' : t.switch_}
            </button>
            <div style={{ flex: 1 }} />
            <button onClick={async () => { setFetching(true); await fetchTargetApi(name); await refresh(); setFetching(false); }} disabled={fetching} style={{ ...S.smallBtn, background: '#064e3b', color: '#6ee7b7' }}>
              {fetching ? '...' : t.fetch}
            </button>
          </div>
          <div style={S.card}>
            <div style={S.statGrid}>
              <Stat label={t.deployed} value={status.deployed?.commit_hash?.substring(0, 7) ?? t.none} />
              <Stat label={t.localHead} value={status.local_commit?.substring(0, 7) ?? '?'} />
              <Stat label={t.remoteHead} value={status.remote_commit?.substring(0, 7) ?? '?'} />
              <Stat label={t.branchLabel} value={status.branch} />
              <Stat label={t.interval} value={`${status.interval_secs}s`} />
              <Stat label={t.mode} value={status.run_mode} />
              <Stat label={t.pid} value={status.pid?.toString() ?? '—'} />
              <Stat label={t.uptime} value={status.uptime_secs != null ? `${status.uptime_secs}s` : '—'} />
              <Stat label={t.healthCheck} value={status.health_status ? (status.health_status.ok ? t.ok : t.fail) : '—'} ok={status.health_status?.ok} />
              <Stat label={t.build} value={status.build_cmd} mono />
              <Stat label={t.run} value={status.run_cmd ?? '—'} mono />
            </div>
          </div>
        </div>
      )}

      {tab === 'commits' && (
        <div style={S.card}>
          {commits.map(c => {
            const deployed = c.hash === status.deployed?.commit_hash;
            return (
              <div key={c.hash} style={S.row}>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                    <code style={S.hashCode}>{c.short_hash}</code>
                    {deployed && <span style={S.deployMark}>● deployed</span>}
                  </div>
                  <div style={{ fontSize: 13, color: '#9ca3af', marginTop: 2, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{c.message}</div>
                </div>
                <div style={{ display: 'flex', gap: 6, flexShrink: 0 }}>
                  <button onClick={() => viewLog(c.hash)} style={S.smallBtn}>{t.log}</button>
                  <button onClick={() => handleRollback(c.hash)} disabled={deployed || loading} style={{ ...S.smallBtn, background: '#450a0a', color: '#fca5a5', opacity: deployed ? 0.4 : 1 }}>
                    {deployed ? t.current : t.rollback}
                  </button>
                </div>
              </div>
            );
          })}
          {commits.length === 0 && <div style={{ textAlign: 'center', color: '#6b7280', padding: 16 }}>{t.noDeployments}</div>}
        </div>
      )}

      {tab === 'history' && (
        <div style={S.card}>
          {[...history].reverse().map((h, i) => (
            <div key={i} style={S.row}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <code style={S.hashCode}>{h.short_hash}</code>
                  <span style={{ fontSize: 12, color: '#6b7280' }}>{new Date(h.deployed_at).toLocaleString()}</span>
                  {h.cache_path && <span style={S.lightTag}>{t.cached}</span>}
                  {h.build_duration_secs != null && <span style={S.lightTag}>{h.build_duration_secs}s</span>}
                </div>
              </div>
              <div style={{ display: 'flex', gap: 6, flexShrink: 0, alignItems: 'center' }}>
                {h.log_path && <button onClick={() => viewLog(h.short_hash)} style={S.smallBtn}>{t.log}</button>}
                <span style={{ fontSize: 12, fontWeight: 600, color: h.success ? '#4ade80' : '#f87171', padding: '2px 8px', borderRadius: 6, background: h.success ? '#052e16' : '#450a0a' }}>{h.success ? t.ok : t.fail}</span>
              </div>
            </div>
          ))}
          {history.length === 0 && <div style={{ textAlign: 'center', color: '#6b7280', padding: 16 }}>{t.noDeployments}</div>}
        </div>
      )}

      {tab === 'config' && <ConfigEditor name={name} />}

      {logHash && (
        <div style={S.card}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 8 }}>
            <span style={{ fontWeight: 600, fontSize: 13, color: '#9ca3af' }}>{t.buildLog} {logHash}</span>
            <button onClick={() => { setLogHash(null); setLog(''); }} style={{ ...S.smallBtn, background: '#1f1f2a', color: '#9ca3af' }}>✕</button>
          </div>
          <pre style={S.buildLog}>{log}</pre>
        </div>
      )}
    </div>
  );
}

function Stat({ label, value, ok, mono }: { label: string; value: string; ok?: boolean; mono?: boolean }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
      <span style={{ fontSize: 11, color: '#6b7280', textTransform: 'uppercase', letterSpacing: '0.05em', fontWeight: 500 }}>{label}</span>
      <span style={{ fontSize: 14, fontFamily: mono ? 'monospace' : 'inherit', color: ok === undefined ? '#d1d5db' : (ok ? '#4ade80' : '#f87171'), fontWeight: ok !== undefined ? 600 : 400 }}>{value}</span>
    </div>
  );
}

function ConfigEditor({ name }: { name: string }) {
  const { t } = useI18n();
  const [config, setConfig] = useState('');
  const [configPath, setConfigPath] = useState('');
  const [mavenSettings, setMavenSettings] = useState('');
  const [mavenSettingsPath, setMavenSettingsPath] = useState('');
  const [viteConfig, setViteConfig] = useState('');
  const [viteConfigPath, setViteConfigPath] = useState('');
  const [jvmArgs, setJvmArgs] = useState('');
  const [envsText, setEnvsText] = useState('');
  const [localRepo, setLocalRepo] = useState('');
  const [subTab, setSubTab] = useState<'config' | 'maven' | 'vite' | 'env' | 'repo'>('config');
  const [saving, setSaving] = useState(false);
  const [statusMsg, setStatusMsg] = useState('');

  const load = useCallback(async () => {
    try {
      const [cfg, ms, vc, env, lr] = await Promise.all([fetchConfig(name), fetchMavenSettings(name), fetchViteConfig(name), fetchEnv(name), fetchLocalRepo(name)]);
      setConfig(cfg.content); setConfigPath(cfg.path);
      setMavenSettings(ms.content); setMavenSettingsPath(ms.path);
      setViteConfig(vc.content); setViteConfigPath(vc.path);
      setJvmArgs(env.jvm_args ?? ''); setEnvsText(Object.entries(env.envs).map(([k, v]) => `${k}=${v}`).join('\n'));
      setLocalRepo(lr.local_repo);
    } catch { /* */ }
  }, [name]);

  useEffect(() => { load(); }, [load]);

  const save = async (fn: () => Promise<void>) => {
    setSaving(true); setStatusMsg('');
    try { await fn(); setStatusMsg(t.saved); } catch (e: any) { setStatusMsg(`${t.errorPrefix} ${e.message || e}`); }
    setSaving(false);
  };

  const cfgTabs = [
    { key: 'config' as const, label: t.configTab },
    { key: 'maven' as const, label: t.mavenSettings },
    { key: 'vite' as const, label: t.viteConfig },
    { key: 'env' as const, label: t.envVarsTab },
    { key: 'repo' as const, label: t.localRepo },
  ];

  return (
    <div>
      <div style={S.tabBar}>
        {cfgTabs.map(ct => (
          <button key={ct.key} onClick={() => setSubTab(ct.key)} style={{ ...S.tab, ...(subTab === ct.key ? S.tabActive : {}) }}>{ct.label}</button>
        ))}
      </div>
      {subTab === 'config' && (
        <div style={S.card}>
          <div style={{ fontSize: 11, color: '#6b7280', marginBottom: 8, fontFamily: 'monospace' }}>{configPath || t.configPath}</div>
          <textarea value={config} onChange={e => setConfig(e.target.value)} style={S.textarea} spellCheck={false} />
          <div style={{ marginTop: 12, display: 'flex', alignItems: 'center', gap: 8 }}>
            <button onClick={() => save(async () => { await saveConfig(name, config); })} disabled={saving} style={S.primaryBtn}>{saving ? t.saving : t.saveConfig}</button>
            <span style={{ fontSize: 13, color: statusMsg.startsWith(t.errorPrefix) ? '#fca5a5' : '#4ade80' }}>{statusMsg}</span>
          </div>
        </div>
      )}
      {subTab === 'maven' && (
        <div style={S.card}>
          <div style={{ fontSize: 11, color: '#6b7280', marginBottom: 8, fontFamily: 'monospace' }}>{mavenSettingsPath || t.noMavenConfigured}</div>
          <textarea value={mavenSettings} onChange={e => setMavenSettings(e.target.value)} style={{ ...S.textarea, minHeight: 400 }} spellCheck={false} />
          <div style={{ marginTop: 12, display: 'flex', alignItems: 'center', gap: 8 }}>
            <button onClick={() => save(async () => { await saveMavenSettings(name, mavenSettings); })} disabled={saving || !mavenSettingsPath} style={{ ...S.primaryBtn, opacity: !mavenSettingsPath ? 0.4 : 1 }}>{saving ? t.saving : t.saveSettings}</button>
            <span style={{ fontSize: 13, color: statusMsg.startsWith(t.errorPrefix) ? '#fca5a5' : '#4ade80' }}>{statusMsg}</span>
          </div>
        </div>
      )}
      {subTab === 'vite' && (
        <div style={S.card}>
          <div style={{ fontSize: 11, color: '#6b7280', marginBottom: 8, fontFamily: 'monospace' }}>{viteConfigPath || t.viteConfigPath}</div>
          <textarea value={viteConfig} onChange={e => setViteConfig(e.target.value)} style={{ ...S.textarea, minHeight: 300 }} spellCheck={false} />
          <div style={{ marginTop: 12, display: 'flex', alignItems: 'center', gap: 8 }}>
            <button onClick={() => save(async () => { await saveViteConfig(name, viteConfig); })} disabled={saving} style={S.primaryBtn}>{saving ? t.saving : t.saveViteConfig}</button>
            <span style={{ fontSize: 13, color: statusMsg.startsWith(t.errorPrefix) ? '#fca5a5' : '#4ade80' }}>{statusMsg}</span>
          </div>
        </div>
      )}
      {subTab === 'env' && (
        <div style={S.card}>
          <div style={{ marginBottom: 16 }}>
            <div style={{ fontSize: 11, color: '#6b7280', textTransform: 'uppercase', letterSpacing: '0.05em', marginBottom: 6 }}>{t.jvmArgsLabel}</div>
            <input value={jvmArgs} onChange={e => setJvmArgs(e.target.value)} style={S.inputField} placeholder={t.jvmArgsPlaceholder} />
          </div>
          <div style={{ marginBottom: 16 }}>
            <div style={{ fontSize: 11, color: '#6b7280', textTransform: 'uppercase', letterSpacing: '0.05em', marginBottom: 6 }}>{t.envVarsLabel}</div>
            <textarea value={envsText} onChange={e => setEnvsText(e.target.value)} style={{ ...S.textarea, minHeight: 150 }} placeholder={t.envVarsPlaceholder} spellCheck={false} />
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <button onClick={() => save(async () => {
              const envs: Record<string, string> = {};
              envsText.split('\n').forEach(line => { const idx = line.indexOf('='); if (idx > 0) envs[line.slice(0, idx).trim()] = line.slice(idx + 1).trim(); });
              await saveEnv(name, jvmArgs || null, envs);
            })} disabled={saving} style={S.primaryBtn}>{saving ? t.saving : t.saveEnvVars}</button>
            <span style={{ fontSize: 13, color: statusMsg.startsWith(t.errorPrefix) ? '#fca5a5' : '#4ade80' }}>{statusMsg}</span>
          </div>
        </div>
      )}
      {subTab === 'repo' && (
        <div style={S.card}>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            <span style={{ fontSize: 11, color: '#6b7280', textTransform: 'uppercase', letterSpacing: '0.05em' }}>{t.localMavenRepo}</span>
            <span style={{ fontFamily: 'monospace', fontSize: 14, color: '#93c5fd' }}>{localRepo || t.notConfigured}</span>
          </div>
          {localRepo && (
            <div style={{ marginTop: 12 }}>
              <a href="#" onClick={e => { e.preventDefault(); navigator.clipboard.writeText(localRepo); }} style={{ fontSize: 13, color: '#6d28d9', textDecoration: 'none', background: '#2e1065', padding: '4px 10px', borderRadius: 6 }}>
                {t.copyPath}
              </a>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function App() { return <I18nProvider><AppInner /></I18nProvider>; }

const S: Record<string, React.CSSProperties> = {
  container: { minHeight: '100vh', background: '#09090b', color: '#d1d5db', fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', sans-serif" },
  header: { padding: '0 20px', height: 52, display: 'flex', alignItems: 'center', gap: 12, background: '#0c0c10', borderBottom: '1px solid #1a1a24', position: 'sticky', top: 0, zIndex: 10 },
  headerLeft: { display: 'flex', alignItems: 'center', gap: 10 },
  logoText: { fontSize: 16, fontWeight: 800, color: '#f1f5f9', letterSpacing: '-0.5px' },
  versionChip: { fontSize: 10, color: '#6b7280', fontFamily: 'monospace', background: '#111118', padding: '1px 6px', borderRadius: 4, border: '1px solid #1f1f2a' },
  buildingBadge: { display: 'flex', alignItems: 'center', gap: 6, padding: '3px 10px', borderRadius: 6, fontSize: 12, background: '#1a1a0a', color: '#fbbf24', border: '1px solid #2d2d10' },
  pulse: { width: 6, height: 6, borderRadius: '50%', background: '#fbbf24', animation: 'pulse 1.5s infinite', display: 'inline-block' },
  updateHint: { fontSize: 11, color: '#fbbf24', fontFamily: 'monospace' },
  langBtn: { padding: '3px 8px', border: '1px solid #1f1f2a', borderRadius: 4, cursor: 'pointer', fontSize: 11, background: '#0c0c10', color: '#6b7280', fontFamily: 'monospace', fontWeight: 600 },
  updateBtn: { padding: '4px 12px', border: '1px solid #1f1f2a', borderRadius: 6, cursor: 'pointer', fontSize: 12, background: '#0c0c10', color: '#9ca3af', fontWeight: 500 },
  progressWrap: { position: 'relative', width: 120, height: 26, background: '#111118', border: '1px solid #1f1f2a', borderRadius: 6, overflow: 'hidden' },
  progressBar: { position: 'absolute', top: 0, left: 0, height: '100%', background: '#065f46', transition: 'width 0.3s' },
  progressText: { position: 'relative', zIndex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', height: '100%', fontSize: 11, color: '#d1d5db', fontFamily: 'monospace' },
  body: { display: 'flex', height: 'calc(100vh - 52px)' },
  sidebar: { width: 272, borderRight: '1px solid #1a1a24', padding: 12, overflow: 'auto', flexShrink: 0, background: '#0c0c10' },
  main: { flex: 1, padding: 20, overflow: 'auto' },
  groupLabel: { fontSize: 10, color: '#4b5563', textTransform: 'uppercase', letterSpacing: '0.1em', fontWeight: 700, padding: '8px 8px 4px' },
  targetRow: { display: 'flex', alignItems: 'center', gap: 4 },
  targetCard: { display: 'block', width: '100%', textAlign: 'left', padding: '10px 10px', border: '1px solid transparent', borderRadius: 8, cursor: 'pointer', background: 'transparent', color: '#d1d5db', marginBottom: 2, transition: 'all 0.15s' },
  targetCardActive: { background: '#13131a', borderColor: '#1f1f2a' },
  targetTop: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 4 },
  targetName: { fontWeight: 600, fontSize: 13, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  targetMeta: { display: 'flex', alignItems: 'center', gap: 6 },
  typeTag: { fontSize: 9, padding: '0 5px', borderRadius: 3, background: '#1f1f2a', color: '#6b7280', fontFamily: 'monospace', fontWeight: 600, textTransform: 'uppercase' },
  targetBranch: { fontSize: 11, color: '#6b7280' },
  targetHash: { fontSize: 10, color: '#4b5563', fontFamily: 'monospace' },
  badgeOn: { display: 'inline-flex', alignItems: 'center', gap: 3, padding: '1px 7px', borderRadius: 4, fontSize: 10, fontWeight: 700, background: '#052e16', color: '#4ade80', whiteSpace: 'nowrap' },
  badgeOff: { display: 'inline-flex', alignItems: 'center', gap: 3, padding: '1px 7px', borderRadius: 4, fontSize: 10, fontWeight: 700, background: '#1f1f1f', color: '#6b7280', whiteSpace: 'nowrap' },
  cloneBtn: { width: 28, height: 28, border: '1px solid #1f1f2a', borderRadius: 6, cursor: 'pointer', fontSize: 14, background: '#0c0c10', color: '#4b5563', display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, transition: 'all 0.15s' },
  detailHead: { display: 'flex', alignItems: 'flex-start', gap: 12, marginBottom: 20, flexWrap: 'wrap' },
  repoTag: { fontSize: 11, color: '#6b7280', background: '#111118', padding: '2px 8px', borderRadius: 4, fontFamily: 'monospace', border: '1px solid #1f1f2a' },
  branchTag: { fontSize: 12, color: '#93c5fd', fontWeight: 500 },
  openBtn: { display: 'inline-flex', alignItems: 'center', padding: '3px 10px', border: '1px solid #1e3a5f', borderRadius: 6, cursor: 'pointer', fontSize: 11, background: '#0c1929', color: '#60a5fa', fontWeight: 500, transition: 'all 0.15s' },
  tabBar: { display: 'flex', gap: 2, marginBottom: 16, background: '#0c0c10', borderRadius: 8, padding: 3, border: '1px solid #1a1a24' },
  tab: { display: 'inline-flex', alignItems: 'center', gap: 6, padding: '7px 14px', border: 'none', borderRadius: 6, cursor: 'pointer', fontSize: 13, background: 'transparent', color: '#6b7280', fontWeight: 500, transition: 'all 0.15s' },
  tabActive: { background: '#13131a', color: '#d1d5db', boxShadow: '0 1px 2px rgba(0,0,0,0.3)' },
  card: { background: '#0c0c10', borderRadius: 10, padding: 16, marginBottom: 12, border: '1px solid #1a1a24' },
  statGrid: { display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(180px, 1fr))', gap: '16px 24px' },
  row: { display: 'flex', alignItems: 'center', gap: 12, padding: '10px 12px', borderRadius: 6, marginBottom: 2, background: '#09090b', border: '1px solid #111118', transition: 'background 0.1s' },
  hashCode: { color: '#93c5fd', fontFamily: 'monospace', fontSize: 12, background: '#111827', padding: '1px 6px', borderRadius: 4 },
  deployMark: { fontSize: 10, color: '#4ade80', fontWeight: 600, background: '#052e16', padding: '1px 6px', borderRadius: 4 },
  lightTag: { fontSize: 10, color: '#60a5fa', background: '#111827', padding: '1px 6px', borderRadius: 4, fontFamily: 'monospace' },
  primaryBtn: { display: 'inline-flex', alignItems: 'center', gap: 6, padding: '7px 16px', border: 'none', borderRadius: 7, cursor: 'pointer', fontSize: 13, fontWeight: 600, background: '#166534', color: '#86efac' },
  smallBtn: { padding: '4px 10px', border: 'none', borderRadius: 6, cursor: 'pointer', fontSize: 11, fontWeight: 500, background: '#1f1f2a', color: '#9ca3af' },
  textarea: { width: '100%', minHeight: 200, background: '#09090b', color: '#d1d5db', border: '1px solid #1f1f2a', borderRadius: 8, padding: 12, fontFamily: "'SF Mono', 'Fira Code', 'Fira Mono', Menlo, monospace", fontSize: 12, resize: 'vertical', lineHeight: 1.6, outline: 'none' },
  inputField: { width: '100%', background: '#09090b', color: '#d1d5db', border: '1px solid #1f1f2a', borderRadius: 8, padding: '8px 12px', fontFamily: "'SF Mono', 'Fira Code', 'Fira Mono', Menlo, monospace", fontSize: 13, outline: 'none' },
  select: { padding: '5px 10px', border: '1px solid #1f1f2a', borderRadius: 6, background: '#09090b', color: '#d1d5db', fontSize: 12, fontFamily: 'monospace', outline: 'none', maxWidth: 200 },
  liveLog: { background: '#09090b', padding: 12, borderRadius: 8, fontSize: 11, fontFamily: "'SF Mono', 'Fira Code', Menlo, monospace", color: '#9ca3af', whiteSpace: 'pre-wrap', maxHeight: 250, overflow: 'auto', margin: 0, lineHeight: 1.5, border: '1px solid #1a1a24' },
  buildLog: { background: '#09090b', padding: 12, borderRadius: 8, fontSize: 11, fontFamily: "'SF Mono', 'Fira Code', Menlo, monospace", color: '#9ca3af', whiteSpace: 'pre-wrap', maxHeight: 400, overflow: 'auto', lineHeight: 1.6, border: '1px solid #1a1a24' },
};

export default App;
