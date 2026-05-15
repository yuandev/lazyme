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
  const wsRef = useRef<WebSocket | null>(null);

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
    wsRef.current = ws;

    ws.onmessage = (ev) => {
      try {
        const data = JSON.parse(ev.data);
        switch (data.event) {
          case 'self_update_checking':
            setUpdate({ phase: 'checking', version: null, progress: 0, error: null });
            break;
          case 'self_update_pulling':
            setUpdate(prev => ({ ...prev, phase: 'downloading', progress: 0 }));
            break;
          case 'self_update_progress':
            setUpdate(prev => ({ ...prev, phase: 'downloading', progress: parseInt(data.message) || 0 }));
            break;
          case 'self_update_complete':
            setUpdate({ phase: 'complete', version: data.commit, progress: 100, error: null });
            break;
          case 'self_update_error':
            setUpdate(prev => ({ ...prev, phase: 'error', error: data.message }));
            break;
          case 'build_log_start':
            setLiveLog({ target: data.target, commit: data.commit });
            setLiveLines([]);
            break;
          case 'build_output':
            setLiveLines(prev => [...prev, data.message || '']);
            break;
          case 'build_log_end':
            setLiveLog(null);
            refreshTargets();
            break;
          case 'build_started':
            setLiveLog({ target: data.target, commit: data.commit || '' });
            setLiveLines([]);
            break;
          case 'build_complete':
            refreshTargets();
            break;
          case 'targets_changed':
          case 'auto_deploy_toggled':
            refreshTargets();
            break;
          default:
            if (data.event?.startsWith('self_update_')) break;
            refreshTargets();
        }
      } catch { /* ignore */ }
    };

    ws.onclose = () => { wsRef.current = null; };
    return () => { ws.close(); };
  }, [refreshTargets]);

  useEffect(() => {
    logEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [liveLines]);

  const triggerSelfUpdate = async () => {
    setUpdate({ phase: 'checking', version: null, progress: 0, error: null });
    const res = await fetch('/api/self-update', { method: 'POST' });
    const data = await res.json();
    if (data.status === 'up_to_date') {
      setUpdate({ phase: null, version: null, progress: 0, error: 'Already up to date' });
    } else if (data.status === 'error') {
      setUpdate({ phase: 'error', version: null, progress: 0, error: data.message });
    }
  };

  const toggleLang = () => setLang(lang === 'en' ? 'zh' : 'en');

  return (
    <div style={s.container}>
      <header style={s.header}>
        <h1 style={s.title}>deployd</h1>
        <span style={s.versionTag}>v{currentVersion}</span>
        {building && <span style={s.buildingBadge}>{t.building} {building}</span>}
        {update.version && !update.phase && (
          <span style={s.updateHint}>{tf(t.versionAvailable, { version: update.version })}</span>
        )}
        <div style={{ flex: 1 }} />
        <button onClick={toggleLang} style={s.langBtn} title="Switch language">
          {lang === 'en' ? '中' : 'EN'}
        </button>
        {update.phase === 'downloading' ? (
          <div style={s.updateBtn}>
            <div style={{ ...s.progressBar, width: `${update.progress}%` }} />
            <span style={s.updateBtnText}>{update.progress}%</span>
          </div>
        ) : update.phase === 'complete' ? (
          <button onClick={restartServer} style={{ ...s.headerBtn, ...s.updateOk }}>
            {t.restartNow}
          </button>
        ) : (
          <button
            onClick={triggerSelfUpdate}
            disabled={!!update.phase}
            style={{
              ...s.headerBtn,
              ...(update.phase === 'error' ? s.updateError : {}),
              ...(update.error === 'Already up to date' ? s.updateOk : {}),
            }}
          >
            {update.phase === 'checking' ? t.checking :
             update.phase === 'error' ? t.error :
             update.version ? tf(t.updateTo, { version: update.version }) :
             update.error ? t.upToDate : t.update}
          </button>
        )}
      </header>
      <div style={s.body}>
        <aside style={s.sidebar}>
          {(() => {
            const groups = new Map<string | null, TargetSummary[]>();
            for (const target of targets) {
              const g = target.group || null;
              if (!groups.has(g)) groups.set(g, []);
              groups.get(g)!.push(target);
            }
            const sorted = [...groups.entries()].sort(([a], [b]) => (a || '~').localeCompare(b || '~'));
            return sorted.map(([group, items]) => (
              <div key={group || '__ungrouped__'} style={{ marginBottom: '0.75rem' }}>
                <div style={s.groupHeader}>{group || t.dash}</div>
                {items.map((target) => (
                  <button
                    key={target.name}
                    onClick={() => setSelected(target.name)}
                    style={{ ...s.targetCard, ...(selected === target.name ? s.targetCardActive : {}) }}
                  >
                    <div style={s.cardName}>
                      {target.name}
                      <span style={target.process_running && target.health_ok !== false ? s.badgeOnline : s.badgeOffline}>
                        {target.process_running && target.health_ok !== false ? t.online : t.offline}
                      </span>
                      <span style={s.serviceBadge}>{target.service_type}</span>
                    </div>
                    <div style={s.cardMeta}>
                      {target.branch} &middot; {target.deployed?.short_hash ?? t.never}
                    </div>
                  </button>
                ))}
              </div>
            ));
          })()}
        </aside>
        <main style={s.main}>
          {liveLog && (
            <div style={s.card}>
              <div style={{ display: 'flex', alignItems: 'center', gap: '0.5rem', marginBottom: '0.5rem' }}>
                <span style={{ fontSize: '0.9rem', fontWeight: 600, color: '#38bdf8' }}>{liveLog.target}</span>
                <code style={{ fontSize: '0.75rem', color: '#64748b' }}>{liveLog.commit}</code>
                <div style={{ flex: 1 }} />
                <button onClick={() => setLiveLog(null)} style={{ ...s.btnLog, fontSize: '0.7rem' }}>x</button>
              </div>
              <pre style={s.liveLog}>
                {liveLines.join('\n')}
                <div ref={logEndRef} />
              </pre>
            </div>
          )}
          {selected ? (
            <TargetDetail name={selected} />
          ) : (
            <div style={s.empty}>{t.selectTarget}</div>
          )}
        </main>
      </div>
    </div>
  );
}

function TargetDetail({ name }: { name: string }) {
  const { t } = useI18n();
  const [tab, setTab] = useState<'status' | 'commits' | 'history' | 'config'>('status');
  const [status, setStatus] = useState<StatusResponse | null>(null);
  const [commits, setCommits] = useState<CommitInfo[]>([]);
  const [history, setHistory] = useState<DeployRecord[]>([]);
  const [log, setLog] = useState<string>('');
  const [logHash, setLogHash] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [branches, setBranches] = useState<string[]>([]);
  const [branchSel, setBranchSel] = useState('');
  const [switchingBranch, setSwitchingBranch] = useState(false);
  const [fetching, setFetching] = useState(false);
  const [cloning, setCloning] = useState(false);

  const refresh = useCallback(async () => {
    const [s, c, h] = await Promise.all([
      fetchTargetStatus(name),
      fetchTargetCommits(name),
      fetchTargetHistory(name),
    ]);
    setStatus(s);
    setCommits(c);
    setHistory(h);
    if (s.branch && !branchSel) setBranchSel(s.branch);
  }, [name]);

  useEffect(() => {
    refresh();
    const timer = setInterval(refresh, REFRESH_MS);
    return () => clearInterval(timer);
  }, [refresh]);

  useEffect(() => {
    fetchBranches(name).then((b) => {
      setBranches(b.branches);
      setBranchSel(b.current);
    });
  }, [name]);

  const viewLog = async (hash: string) => {
    setLogHash(hash);
    const content = await fetchTargetLogs(name, hash);
    setLog(content || t.noLog);
  };

  const handleDeploy = async () => {
    setLoading(true);
    await deployTarget(name);
    await refresh();
    setLoading(false);
  };

  const handleRollback = async (commit: string) => {
    setLoading(true);
    await rollbackTarget(name, commit);
    await refresh();
    setLoading(false);
  };

  const handleSwitchBranch = async () => {
    if (!branchSel || branchSel === status?.branch) return;
    setSwitchingBranch(true);
    await switchBranch(name, branchSel);
    await refresh();
    setSwitchingBranch(false);
  };

  const handleFetch = async () => {
    setFetching(true);
    await fetchTargetApi(name);
    await refresh();
    setFetching(false);
  };

  const handleClone = async () => {
    const newName = prompt(t.cloneName, `${name}-clone`);
    if (!newName) return;
    const newRepo = prompt(t.cloneRepo, status?.repo ?? '');
    if (newRepo === null) return;
    setCloning(true);
    try {
      await cloneTarget(name, newName, newRepo.trim() || undefined);
    } catch (e: any) {
      alert(`${t.cloneFailed} ${e.message || e}`);
    }
    setCloning(false);
  };

  if (!status) return <div style={s.empty}>{t.loading}</div>;

  const tabLabels: Record<string, string> = {
    status: t.status,
    commits: t.commits,
    history: t.history,
    config: t.config,
  };

  return (
    <div>
      <div style={s.detailHeader}>
        <h2 style={s.detailName}>{status.name}</h2>
        <span style={s.detailRepo}>{status.repo}</span>
        <span style={s.detailBranch}>@{status.branch}</span>
        <span style={status.process_running && status.health_status?.ok !== false ? s.badgeOnline : s.badgeOffline}>
          {status.process_running && status.health_status?.ok !== false ? t.online : t.offline}
        </span>
        {status.health_url && (
          <>
            <span style={s.badgeCache}>{t.health} {status.health_url}</span>
            <button
              onClick={() => {
                try {
                  const u = new URL(status.health_url!);
                  window.open(u.origin, '_blank');
                } catch { /* invalid url */ }
              }}
              title={t.openTitle}
              style={s.btnOpen}
            >{t.open}</button>
          </>
        )}
        {status.run_cmd && (
          <span style={s.runCmd} title={status.run_cmd}>{status.run_cmd}</span>
        )}
      </div>

      <div style={s.tabs}>
        {(['status', 'commits', 'history', 'config'] as const).map((tabKey) => (
          <button
            key={tabKey}
            onClick={() => setTab(tabKey)}
            style={{ ...s.tab, ...(tab === tabKey ? s.tabActive : {}) }}
          >
            {tabLabels[tabKey]}
          </button>
        ))}
      </div>

      {tab === 'status' && (
        <div>
          <div style={s.actions}>
            <button onClick={handleDeploy} disabled={loading} style={s.btnPrimary}>
              {loading ? t.deploying : t.deployLatest}
            </button>
            <button
              onClick={async () => { await autoDeployToggle(name); await refresh(); }}
              style={{
                ...s.btnPrimary,
                background: status.auto_deploy_paused ? '#166534' : '#713f12',
                color: status.auto_deploy_paused ? '#4ade80' : '#facc15',
              }}
            >
              {status.auto_deploy_paused ? t.resume : t.pause}
            </button>
            {status.auto_deploy_paused && (
              <span style={{ fontSize: '0.8rem', color: '#f59e0b', marginLeft: '0.5rem' }}>
                {t.autoDeployPaused}
              </span>
            )}
          </div>
          <div style={{ ...s.card, marginBottom: '0.75rem', display: 'flex', alignItems: 'center', gap: '0.5rem' }}>
            <span style={{ fontSize: '0.75rem', color: '#94a3b8', flexShrink: 0 }}>{t.branch}</span>
            <select
              value={branchSel}
              onChange={(e) => setBranchSel(e.target.value)}
              style={s.branchSelect}
            >
              {branches.length === 0 && (
                <option value={status.branch}>{status.branch}</option>
              )}
              {branches.map((b) => (
                <option key={b} value={b}>{b}</option>
              ))}
            </select>
            <button
              onClick={handleSwitchBranch}
              disabled={switchingBranch || !branchSel || branchSel === status.branch}
              style={{ ...s.btnSwitch, ...(branchSel === status.branch ? s.btnDisabled : {}) }}
            >
              {switchingBranch ? '...' : t.switch_}
            </button>
            <div style={{ flex: 1 }} />
            <button onClick={handleFetch} disabled={fetching} style={s.btnFetch}>
              {fetching ? '...' : t.fetch}
            </button>
            <button onClick={handleClone} disabled={cloning} style={s.btnClone}>
              {t.clone}
            </button>
          </div>
          <div style={s.card}>
            <div style={s.grid}>
              <Item label={t.deployed} value={status.deployed?.commit_hash?.substring(0, 7) ?? t.none} />
              <Item label={t.localHead} value={status.local_commit?.substring(0, 7) ?? t.unknown} />
              <Item label={t.remoteHead} value={status.remote_commit?.substring(0, 7) ?? t.unknown} />
              <Item label={t.branchLabel} value={status.branch} />
              <Item label={t.interval} value={`${status.interval_secs}${t.seconds}`} />
              <Item label={t.mode} value={status.run_mode} />
              <Item label={t.pid} value={status.pid?.toString() ?? t.dash} />
              <Item label={t.uptime} value={status.uptime_secs != null ? `${status.uptime_secs}${t.seconds}` : t.dash} />
              <Item label={t.healthCheck} value={status.health_status ? `${status.health_status.ok ? t.ok : t.fail}` : t.dash} />
              <Item label={t.build} value={status.build_cmd} />
              <Item label={t.run} value={status.run_cmd ?? t.dash} />
              <Item label={t.jvmArgs} value={status.jvm_args ?? t.dash} />
              <Item label={t.envVars} value={Object.keys(status.envs).length > 0 ? Object.keys(status.envs).join(', ') : t.dash} />
            </div>
          </div>
        </div>
      )}

      {tab === 'commits' && (
        <div style={s.card}>
          {commits.map((c) => {
            const isDeployed = c.hash === status.deployed?.commit_hash;
            return (
              <div key={c.hash} style={s.listItem}>
                <code style={s.hash}>{c.short_hash}</code>
                <span style={s.msg}>{c.message}</span>
                <button onClick={() => viewLog(c.hash)} style={s.btnLog}>{t.log}</button>
                <button
                  onClick={() => handleRollback(c.hash)}
                  disabled={isDeployed || loading}
                  style={{ ...s.btnRollback, ...(isDeployed ? s.btnDisabled : {}) }}
                >
                  {isDeployed ? t.current : t.deploy}
                </button>
              </div>
            );
          })}
        </div>
      )}

      {tab === 'history' && (
        <div style={s.card}>
          {[...history].reverse().length === 0 ? (
            <div style={s.empty}>{t.noDeployments}</div>
          ) : (
            [...history].reverse().map((h, i) => (
              <div key={i} style={s.listItem}>
                <code style={s.hash}>{h.short_hash}</code>
                <span style={s.msg}>{new Date(h.deployed_at).toLocaleString()}</span>
                {h.cache_path && <span style={s.badgeCache}>{t.cached}</span>}
                {h.log_path && (
                  <button onClick={() => viewLog(h.short_hash)} style={s.btnLog}>{t.log}</button>
                )}
                {h.build_duration_secs != null && (
                  <span style={s.badgeCache}>{h.build_duration_secs}s</span>
                )}
                <span style={h.success ? s.badgeGreen : s.badgeYellow}>
                  {h.success ? t.ok : t.fail}
                </span>
              </div>
            ))
          )}
        </div>
      )}

      {tab === 'config' && (
        <ConfigEditor name={name} />
      )}

      {logHash && (
        <div style={s.card}>
          <h3 style={s.cardTitle}>{t.buildLog} {logHash}</h3>
          <pre style={s.log}>{log}</pre>
        </div>
      )}
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
      const [cfg, ms, vc, env, lr] = await Promise.all([
        fetchConfig(name),
        fetchMavenSettings(name),
        fetchViteConfig(name),
        fetchEnv(name),
        fetchLocalRepo(name),
      ]);
      setConfig(cfg.content);
      setConfigPath(cfg.path);
      setMavenSettings(ms.content);
      setMavenSettingsPath(ms.path);
      setViteConfig(vc.content);
      setViteConfigPath(vc.path);
      setJvmArgs(env.jvm_args ?? '');
      setEnvsText(Object.entries(env.envs).map(([k, v]) => `${k}=${v}`).join('\n'));
      setLocalRepo(lr.local_repo);
    } catch { /* config may not exist yet */ }
  }, [name]);

  useEffect(() => { load(); }, [load]);

  const save = async (fn: () => Promise<void>) => {
    setSaving(true);
    setStatusMsg('');
    try {
      await fn();
      setStatusMsg(t.saved);
    } catch (e: any) {
      setStatusMsg(`${t.errorPrefix} ${e.message || e}`);
    }
    setSaving(false);
  };

  const styles = {
    textarea: { width: '100%', minHeight: 200, background: '#0f172a', color: '#e2e8f0', border: '1px solid #334155', borderRadius: 6, padding: '0.75rem', fontFamily: 'monospace', fontSize: '0.8rem', resize: 'vertical' as const },
    input: { width: '100%', background: '#0f172a', color: '#e2e8f0', border: '1px solid #334155', borderRadius: 6, padding: '0.5rem 0.75rem', fontFamily: 'monospace', fontSize: '0.85rem' },
    btn: { ...s.btnPrimary, padding: '0.4rem 0.9rem', fontSize: '0.8rem' },
    subTab: (tab: string) => ({ ...s.tab, ...(subTab === tab ? s.tabActive : {}) }),
    pathLabel: { fontSize: '0.7rem', color: '#64748b', marginBottom: '0.5rem', fontFamily: 'monospace' },
    status: { fontSize: '0.8rem', color: statusMsg.startsWith(t.errorPrefix) ? '#fca5a5' : '#4ade80', marginLeft: '0.75rem' },
  };

  return (
    <div>
      <div style={{ ...s.tabs, marginBottom: '0.75rem' }}>
        <button onClick={() => setSubTab('config')} style={styles.subTab('config')}>{t.configTab}</button>
        <button onClick={() => setSubTab('maven')} style={styles.subTab('maven')}>{t.mavenSettings}</button>
        <button onClick={() => setSubTab('vite')} style={styles.subTab('vite')}>{t.viteConfig}</button>
        <button onClick={() => setSubTab('env')} style={styles.subTab('env')}>{t.envVarsTab}</button>
        <button onClick={() => setSubTab('repo')} style={styles.subTab('repo')}>{t.localRepo}</button>
      </div>

      {subTab === 'config' && (
        <div style={s.card}>
          <div style={styles.pathLabel}>{configPath || t.configPath}</div>
          <textarea value={config} onChange={(e) => setConfig(e.target.value)} style={styles.textarea} spellCheck={false} />
          <div style={{ marginTop: '0.75rem', display: 'flex', alignItems: 'center', gap: '0.5rem' }}>
            <button onClick={() => save(async () => { await saveConfig(name, config); })} disabled={saving} style={styles.btn}>
              {saving ? t.saving : t.saveConfig}
            </button>
            <span style={styles.status}>{statusMsg}</span>
          </div>
        </div>
      )}

      {subTab === 'maven' && (
        <div style={s.card}>
          <div style={styles.pathLabel}>{mavenSettingsPath || t.noMavenConfigured}</div>
          <textarea value={mavenSettings} onChange={(e) => setMavenSettings(e.target.value)} style={{ ...styles.textarea, minHeight: 400 }} spellCheck={false} />
          <div style={{ marginTop: '0.75rem', display: 'flex', alignItems: 'center', gap: '0.5rem' }}>
            <button
              onClick={() => save(async () => { await saveMavenSettings(name, mavenSettings); })}
              disabled={saving || !mavenSettingsPath}
              style={{ ...styles.btn, ...(!mavenSettingsPath ? s.btnDisabled : {}) }}
            >
              {saving ? t.saving : t.saveSettings}
            </button>
            <span style={styles.status}>{statusMsg}</span>
          </div>
        </div>
      )}

      {subTab === 'vite' && (
        <div style={s.card}>
          <div style={styles.pathLabel}>{viteConfigPath || t.viteConfigPath}</div>
          <textarea value={viteConfig} onChange={(e) => setViteConfig(e.target.value)} style={{ ...styles.textarea, minHeight: 300 }} spellCheck={false} />
          <div style={{ marginTop: '0.75rem', display: 'flex', alignItems: 'center', gap: '0.5rem' }}>
            <button onClick={() => save(async () => { await saveViteConfig(name, viteConfig); })} disabled={saving} style={styles.btn}>
              {saving ? t.saving : t.saveViteConfig}
            </button>
            <span style={styles.status}>{statusMsg}</span>
          </div>
        </div>
      )}

      {subTab === 'env' && (
        <div style={s.card}>
          <div style={{ marginBottom: '1rem' }}>
            <div style={s.label}>{t.jvmArgsLabel}</div>
            <input value={jvmArgs} onChange={(e) => setJvmArgs(e.target.value)} style={styles.input} placeholder={t.jvmArgsPlaceholder} />
          </div>
          <div style={{ marginBottom: '1rem' }}>
            <div style={s.label}>{t.envVarsLabel}</div>
            <textarea value={envsText} onChange={(e) => setEnvsText(e.target.value)} style={{ ...styles.textarea, minHeight: 150 }} placeholder={t.envVarsPlaceholder} spellCheck={false} />
          </div>
          <div style={{ marginTop: '0.75rem', display: 'flex', alignItems: 'center', gap: '0.5rem' }}>
            <button
              onClick={() => save(async () => {
                const envs: Record<string, string> = {};
                envsText.split('\n').forEach(line => {
                  const idx = line.indexOf('=');
                  if (idx > 0) envs[line.slice(0, idx).trim()] = line.slice(idx + 1).trim();
                });
                await saveEnv(name, jvmArgs || null, envs);
              })}
              disabled={saving}
              style={styles.btn}
            >
              {saving ? t.saving : t.saveEnvVars}
            </button>
            <span style={styles.status}>{statusMsg}</span>
          </div>
        </div>
      )}

      {subTab === 'repo' && (
        <div style={s.card}>
          <div style={s.grid}>
            <div style={s.itemWrap}>
              <span style={s.label}>{t.localMavenRepo}</span>
              <span style={{ ...s.value, fontFamily: 'monospace', fontSize: '0.8rem', color: '#38bdf8' }}>
                {localRepo || t.notConfigured}
              </span>
            </div>
          </div>
          {localRepo && (
            <div style={{ marginTop: '0.75rem', fontSize: '0.75rem', color: '#64748b' }}>
              <a href={`#`} onClick={(e) => { e.preventDefault(); navigator.clipboard.writeText(localRepo); }} style={{ color: '#7dd3fc', textDecoration: 'none' }}>
                {t.copyPath}
              </a>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function Item({ label, value }: { label: string; value: string }) {
  return (
    <div style={s.itemWrap}>
      <span style={s.label}>{label}</span>
      <span style={s.value}>{value}</span>
    </div>
  );
}

function App() {
  return (
    <I18nProvider>
      <AppInner />
    </I18nProvider>
  );
}

const s: Record<string, React.CSSProperties> = {
  container: { minHeight: '100vh', background: '#0f172a', color: '#e2e8f0', fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif' },
  header: { padding: '1rem 2rem', borderBottom: '1px solid #1e293b', display: 'flex', alignItems: 'center', gap: '1rem' },
  title: { fontSize: '1.25rem', color: '#38bdf8', margin: 0 },
  buildingBadge: { padding: '0.2rem 0.6rem', borderRadius: 999, fontSize: '0.75rem', background: '#713f12', color: '#facc15' },
  versionTag: { fontSize: '0.7rem', color: '#64748b', fontFamily: 'monospace', background: '#1e293b', padding: '0.15rem 0.4rem', borderRadius: 4 },
  updateHint: { fontSize: '0.7rem', color: '#facc15', fontFamily: 'monospace' },
  headerBtn: { padding: '0.3rem 0.8rem', border: '1px solid #334155', borderRadius: 6, cursor: 'pointer', fontSize: '0.75rem', background: '#1e293b', color: '#94a3b8' },
  langBtn: { padding: '0.2rem 0.5rem', border: '1px solid #334155', borderRadius: 4, cursor: 'pointer', fontSize: '0.7rem', background: '#1e293b', color: '#94a3b8', fontFamily: 'monospace' },
  updateBtn: { position: 'relative', width: 120, height: 26, background: '#1e293b', border: '1px solid #334155', borderRadius: 6, overflow: 'hidden' },
  progressBar: { position: 'absolute', top: 0, left: 0, height: '100%', background: '#166534', transition: 'width 0.2s' },
  updateBtnText: { position: 'relative', zIndex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', height: '100%', fontSize: '0.7rem', color: '#e2e8f0', fontFamily: 'monospace' },
  updateError: { background: '#7f1d1d', borderColor: '#991b1b', color: '#fca5a5' },
  updateOk: { borderColor: '#166534', color: '#4ade80' },
  body: { display: 'flex', height: 'calc(100vh - 60px)' },
  sidebar: { width: 260, borderRight: '1px solid #1e293b', padding: '1rem', overflow: 'auto', flexShrink: 0 },
  main: { flex: 1, padding: '1.5rem', overflow: 'auto' },
  targetCard: { display: 'block', width: '100%', textAlign: 'left', padding: '0.75rem 1rem', border: 'none', borderRadius: 6, cursor: 'pointer', background: 'transparent', color: '#e2e8f0', marginBottom: '0.25rem' },
  targetCardActive: { background: '#1e293b' },
  cardName: { fontWeight: 600, fontSize: '0.9rem', display: 'flex', alignItems: 'center', gap: '0.4rem' },
  cardMeta: { fontSize: '0.75rem', color: '#64748b', marginTop: '0.2rem' },
  dot: { width: 7, height: 7, borderRadius: '50%', background: '#4ade80', display: 'inline-block', flexShrink: 0 },
  groupHeader: { fontSize: '0.65rem', color: '#475569', textTransform: 'uppercase', letterSpacing: '0.05em', padding: '0.5rem 1rem 0.25rem', fontWeight: 600 },
  serviceBadge: { fontSize: '0.6rem', padding: '0.1rem 0.35rem', borderRadius: 3, background: '#334155', color: '#94a3b8', marginLeft: '0.4rem', fontFamily: 'monospace', fontWeight: 400 },
  detailHeader: { display: 'flex', alignItems: 'center', gap: '0.75rem', flexWrap: 'wrap', marginBottom: '1rem' },
  detailName: { fontSize: '1.2rem', margin: 0, color: '#e2e8f0' },
  detailRepo: { fontSize: '0.75rem', color: '#64748b', fontFamily: 'monospace' },
  detailBranch: { fontSize: '0.75rem', color: '#38bdf8' },
  tabs: { display: 'flex', gap: '0.25rem', marginBottom: '1rem' },
  tab: { padding: '0.4rem 1rem', border: 'none', borderRadius: 6, cursor: 'pointer', fontSize: '0.85rem', background: 'transparent', color: '#94a3b8' },
  tabActive: { background: '#1e293b', color: '#e2e8f0' },
  card: { background: '#1e293b', borderRadius: 8, padding: '1.25rem', marginBottom: '1rem' },
  cardTitle: { fontSize: '0.9rem', color: '#94a3b8', margin: 0, marginBottom: '0.5rem' },
  grid: { display: 'flex', gap: '2rem', flexWrap: 'wrap' },
  itemWrap: { display: 'flex', flexDirection: 'column', gap: '0.25rem' },
  label: { fontSize: '0.7rem', color: '#64748b', textTransform: 'uppercase' },
  value: { fontSize: '0.9rem' },
  listItem: { display: 'flex', alignItems: 'center', gap: '0.75rem', padding: '0.5rem 0.75rem', background: '#0f172a', borderRadius: 6, fontSize: '0.85rem', marginBottom: '0.3rem' },
  hash: { color: '#38bdf8', fontFamily: 'monospace', fontSize: '0.8rem' },
  msg: { flex: 1, color: '#94a3b8', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  actions: { marginBottom: '1rem' },
  btnPrimary: { padding: '0.5rem 1.25rem', border: 'none', borderRadius: 6, cursor: 'pointer', fontSize: '0.85rem', fontWeight: 500, background: '#166534', color: '#4ade80' },
  btnRollback: { padding: '0.3rem 0.7rem', border: 'none', borderRadius: 4, cursor: 'pointer', fontSize: '0.8rem', background: '#b91c1c', color: '#fff', flexShrink: 0 },
  btnLog: { padding: '0.3rem 0.7rem', border: 'none', borderRadius: 4, cursor: 'pointer', fontSize: '0.8rem', background: '#1e3a5f', color: '#7dd3fc', flexShrink: 0 },
  btnDisabled: { opacity: 0.4, cursor: 'default' },
  badgeGreen: { display: 'inline-block', padding: '0.15rem 0.5rem', borderRadius: 999, fontSize: '0.7rem', background: '#166534', color: '#4ade80' },
  badgeYellow: { display: 'inline-block', padding: '0.15rem 0.5rem', borderRadius: 999, fontSize: '0.7rem', background: '#713f12', color: '#facc15' },
  badgeOnline: { display: 'inline-block', padding: '0.1rem 0.5rem', borderRadius: 999, fontSize: '0.65rem', background: '#166534', color: '#4ade80', fontWeight: 600, marginLeft: '0.5rem' },
  badgeOffline: { display: 'inline-block', padding: '0.1rem 0.5rem', borderRadius: 999, fontSize: '0.65rem', background: '#7f1d1d', color: '#fca5a5', fontWeight: 600, marginLeft: '0.5rem' },
  badgeCache: { display: 'inline-block', padding: '0.15rem 0.5rem', borderRadius: 999, fontSize: '0.7rem', background: '#1e3a5f', color: '#7dd3fc' },
  empty: { color: '#64748b', padding: '2rem', textAlign: 'center' },
  log: { background: '#0f172a', padding: '1rem', borderRadius: 6, fontSize: '0.75rem', fontFamily: 'monospace', color: '#94a3b8', whiteSpace: 'pre-wrap', maxHeight: 400, overflow: 'auto' },
  liveLog: { background: '#0f172a', padding: '0.75rem', borderRadius: 6, fontSize: '0.7rem', fontFamily: 'monospace', color: '#94a3b8', whiteSpace: 'pre-wrap', maxHeight: 250, overflow: 'auto', margin: 0 },
  branchSelect: { padding: '0.3rem 0.5rem', border: '1px solid #334155', borderRadius: 4, background: '#0f172a', color: '#e2e8f0', fontSize: '0.8rem', fontFamily: 'monospace', maxWidth: 180 },
  btnSwitch: { padding: '0.3rem 0.7rem', border: 'none', borderRadius: 4, cursor: 'pointer', fontSize: '0.75rem', background: '#1e3a5f', color: '#7dd3fc', flexShrink: 0 },
  btnFetch: { padding: '0.3rem 0.7rem', border: 'none', borderRadius: 4, cursor: 'pointer', fontSize: '0.75rem', background: '#166534', color: '#4ade80', flexShrink: 0 },
  btnClone: { padding: '0.3rem 0.7rem', border: 'none', borderRadius: 4, cursor: 'pointer', fontSize: '0.75rem', background: '#581c87', color: '#c084fc', flexShrink: 0 },
  btnOpen: { padding: '0.2rem 0.6rem', border: '1px solid #166534', borderRadius: 4, cursor: 'pointer', fontSize: '0.7rem', background: '#14532d', color: '#4ade80', flexShrink: 0 },
  runCmd: { fontSize: '0.7rem', color: '#64748b', fontFamily: 'monospace', maxWidth: 320, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', flexShrink: 0 },
};

export default App;
