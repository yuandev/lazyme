import { useState, useEffect, useCallback, useRef } from 'react';
import {
  fetchTargets, fetchTargetStatus, fetchTargetCommits, fetchTargetHistory,
  fetchTargetLogs, deployTarget, rollbackTarget, switchBranch, fetchQueue,
} from './api';
import type { TargetSummary, StatusResponse, CommitInfo, DeployRecord } from './api';

const REFRESH_MS = 8_000;

type UpdatePhase = null | 'checking' | 'pulling' | 'building' | 'complete' | 'error';
interface UpdateState {
  phase: UpdatePhase;
  commit: string | null;
  message: string | null;
}

function App() {
  const [targets, setTargets] = useState<TargetSummary[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [building, setBuilding] = useState<string | null>(null);
  const [update, setUpdate] = useState<UpdateState>({ phase: null, commit: null, message: null });
  const wsRef = useRef<WebSocket | null>(null);

  const refreshTargets = useCallback(async () => {
    const list = await fetchTargets();
    setTargets(list);
    const q = await fetchQueue();
    setBuilding(q.building);
  }, []);

  useEffect(() => {
    refreshTargets();
    const timer = setInterval(refreshTargets, REFRESH_MS);
    return () => clearInterval(timer);
  }, [refreshTargets]);

  // WebSocket connection
  useEffect(() => {
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    const ws = new WebSocket(`${proto}://${location.host}/ws`);
    wsRef.current = ws;

    ws.onmessage = (ev) => {
      try {
        const data = JSON.parse(ev.data);
        if (data.event?.startsWith('self_update_')) {
          setUpdate({
            phase: data.event.replace('self_update_', '') as UpdatePhase,
            commit: data.commit ?? null,
            message: data.message ?? null,
          });
          if (data.event !== 'self_update_error' && data.event !== 'self_update_complete') {
            refreshTargets();
          }
        } else {
          refreshTargets();
        }
      } catch {
        // ignore malformed messages
      }
    };

    ws.onclose = () => { wsRef.current = null; };

    return () => { ws.close(); };
  }, [refreshTargets]);

  const triggerSelfUpdate = async () => {
    setUpdate({ phase: 'checking', commit: null, message: null });
    const res = await fetch('/api/self-update', { method: 'POST' });
    const data = await res.json();
    if (data.status === 'up_to_date') {
      setUpdate({ phase: null, commit: null, message: 'Already up to date' });
    } else if (data.status === 'error') {
      setUpdate({ phase: 'error', commit: null, message: data.message });
    }
    // 'updating' — WS events will drive the UI
  };

  const updateLabel = () => {
    if (!update.phase) return update.message ? '✓' : 'update';
    switch (update.phase) {
      case 'checking': return 'checking...';
      case 'pulling': return 'pulling...';
      case 'building': return 'building...';
      case 'complete': return 'restarting...';
      case 'error': return 'error';
    }
  };

  const updateStyle = (): React.CSSProperties => {
    const base = { ...s.headerBtn };
    if (update.phase === 'error') return { ...base, background: '#7f1d1d', color: '#fca5a5' };
    if (update.phase && update.phase !== 'complete') return { ...base, background: '#713f12', color: '#facc15' };
    if (update.phase === 'complete') return { ...base, background: '#166534', color: '#4ade80' };
    if (update.message === 'Already up to date') return { ...base, color: '#4ade80' };
    return base;
  };

  return (
    <div style={s.container}>
      <header style={s.header}>
        <h1 style={s.title}>deployd</h1>
        {building && <span style={s.buildingBadge}>building: {building}</span>}
        <div style={{ flex: 1 }} />
        <button
          onClick={triggerSelfUpdate}
          disabled={!!update.phase}
          style={updateStyle()}
        >
          {updateLabel()}
        </button>
      </header>
      <div style={s.body}>
        <aside style={s.sidebar}>
          {targets.map((t) => (
            <button
              key={t.name}
              onClick={() => setSelected(t.name)}
              style={{ ...s.targetCard, ...(selected === t.name ? s.targetCardActive : {}) }}
            >
              <div style={s.cardName}>
                {t.process_running && <span style={s.dot} />}
                {t.name}
              </div>
              <div style={s.cardMeta}>
                {t.branch} &middot; {t.deployed?.short_hash ?? 'never'}
              </div>
            </button>
          ))}
        </aside>
        <main style={s.main}>
          {selected ? (
            <TargetDetail name={selected} />
          ) : (
            <div style={s.empty}>Select a target</div>
          )}
        </main>
      </div>
    </div>
  );
}

function TargetDetail({ name }: { name: string }) {
  const [tab, setTab] = useState<'status' | 'commits' | 'history'>('status');
  const [status, setStatus] = useState<StatusResponse | null>(null);
  const [commits, setCommits] = useState<CommitInfo[]>([]);
  const [history, setHistory] = useState<DeployRecord[]>([]);
  const [log, setLog] = useState<string>('');
  const [logHash, setLogHash] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [branchInput, setBranchInput] = useState('');
  const [switchingBranch, setSwitchingBranch] = useState(false);

  const refresh = useCallback(async () => {
    const [s, c, h] = await Promise.all([
      fetchTargetStatus(name),
      fetchTargetCommits(name),
      fetchTargetHistory(name),
    ]);
    setStatus(s);
    setCommits(c);
    setHistory(h);
  }, [name]);

  useEffect(() => {
    refresh();
    const timer = setInterval(refresh, REFRESH_MS);
    return () => clearInterval(timer);
  }, [refresh]);

  const viewLog = async (hash: string) => {
    setLogHash(hash);
    const content = await fetchTargetLogs(name, hash);
    setLog(content || '(no log)');
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
    if (!branchInput.trim()) return;
    setSwitchingBranch(true);
    await switchBranch(name, branchInput.trim());
    setBranchInput('');
    await refresh();
    setSwitchingBranch(false);
  };

  if (!status) return <div style={s.empty}>Loading...</div>;

  return (
    <div>
      <div style={s.detailHeader}>
        <h2 style={s.detailName}>{status.name}</h2>
        <span style={s.detailRepo}>{status.repo}</span>
        <span style={s.detailBranch}>@{status.branch}</span>
        {status.process_running && <span style={s.badgeGreen}>running</span>}
        {status.health_url && <span style={s.badgeCache}>health: {status.health_url}</span>}
      </div>

      <div style={s.tabs}>
        {(['status', 'commits', 'history'] as const).map((t) => (
          <button
            key={t}
            onClick={() => setTab(t)}
            style={{ ...s.tab, ...(tab === t ? s.tabActive : {}) }}
          >
            {t === 'status' ? 'Status' : t === 'commits' ? 'Commits' : 'History'}
          </button>
        ))}
      </div>

      {tab === 'status' && (
        <div>
          <div style={s.actions}>
            <button onClick={handleDeploy} disabled={loading} style={s.btnPrimary}>
              {loading ? 'Deploying...' : 'Deploy Latest'}
            </button>
          </div>
          <div style={{ ...s.card, marginBottom: '0.75rem' }}>
            <span style={{ fontSize: '0.75rem', color: '#94a3b8', marginRight: '0.5rem' }}>Branch:</span>
            <input
              placeholder={status.branch}
              value={branchInput}
              onChange={(e) => setBranchInput(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && handleSwitchBranch()}
              style={s.branchInput}
            />
            <button
              onClick={handleSwitchBranch}
              disabled={switchingBranch || !branchInput.trim()}
              style={s.btnSwitch}
            >
              {switchingBranch ? '...' : 'switch'}
            </button>
          </div>
          <div style={s.card}>
            <div style={s.grid}>
              <Item label="Deployed" value={status.deployed?.commit_hash?.substring(0, 7) ?? 'none'} />
              <Item label="Local HEAD" value={status.local_commit?.substring(0, 7) ?? '?'} />
              <Item label="Remote HEAD" value={status.remote_commit?.substring(0, 7) ?? '?'} />
              <Item label="Branch" value={status.branch} />
              <Item label="Interval" value={`${status.interval_secs}s`} />
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
                <button
                  onClick={() => viewLog(c.hash)}
                  style={s.btnLog}
                >log</button>
                <button
                  onClick={() => handleRollback(c.hash)}
                  disabled={isDeployed || loading}
                  style={{ ...s.btnRollback, ...(isDeployed ? s.btnDisabled : {}) }}
                >
                  {isDeployed ? 'current' : 'rollback'}
                </button>
              </div>
            );
          })}
        </div>
      )}

      {tab === 'history' && (
        <div style={s.card}>
          {[...history].reverse().length === 0 ? (
            <div style={s.empty}>No deployments yet</div>
          ) : (
            [...history].reverse().map((h, i) => (
              <div key={i} style={s.listItem}>
                <code style={s.hash}>{h.short_hash}</code>
                <span style={s.msg}>{new Date(h.deployed_at).toLocaleString()}</span>
                {h.cache_path && <span style={s.badgeCache}>cached</span>}
                {h.log_path && (
                  <button onClick={() => viewLog(h.short_hash)} style={s.btnLog}>log</button>
                )}
                <span style={h.success ? s.badgeGreen : s.badgeYellow}>
                  {h.success ? 'ok' : 'fail'}
                </span>
              </div>
            ))
          )}
        </div>
      )}

      {logHash && (
        <div style={s.card}>
          <h3 style={s.cardTitle}>Build Log: {logHash}</h3>
          <pre style={s.log}>{log}</pre>
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

const s: Record<string, React.CSSProperties> = {
  container: { minHeight: '100vh', background: '#0f172a', color: '#e2e8f0', fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif' },
  header: { padding: '1rem 2rem', borderBottom: '1px solid #1e293b', display: 'flex', alignItems: 'center', gap: '1rem' },
  title: { fontSize: '1.25rem', color: '#38bdf8', margin: 0 },
  buildingBadge: { padding: '0.2rem 0.6rem', borderRadius: 999, fontSize: '0.75rem', background: '#713f12', color: '#facc15' },
  headerBtn: { padding: '0.3rem 0.8rem', border: '1px solid #334155', borderRadius: 6, cursor: 'pointer', fontSize: '0.75rem', background: '#1e293b', color: '#94a3b8' },
  body: { display: 'flex', height: 'calc(100vh - 60px)' },
  sidebar: { width: 260, borderRight: '1px solid #1e293b', padding: '1rem', overflow: 'auto', flexShrink: 0 },
  main: { flex: 1, padding: '1.5rem', overflow: 'auto' },
  targetCard: { display: 'block', width: '100%', textAlign: 'left', padding: '0.75rem 1rem', border: 'none', borderRadius: 6, cursor: 'pointer', background: 'transparent', color: '#e2e8f0', marginBottom: '0.25rem' },
  targetCardActive: { background: '#1e293b' },
  cardName: { fontWeight: 600, fontSize: '0.9rem', display: 'flex', alignItems: 'center', gap: '0.4rem' },
  cardMeta: { fontSize: '0.75rem', color: '#64748b', marginTop: '0.2rem' },
  dot: { width: 7, height: 7, borderRadius: '50%', background: '#4ade80', display: 'inline-block', flexShrink: 0 },
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
  badgeCache: { display: 'inline-block', padding: '0.15rem 0.5rem', borderRadius: 999, fontSize: '0.7rem', background: '#1e3a5f', color: '#7dd3fc' },
  empty: { color: '#64748b', padding: '2rem', textAlign: 'center' },
  log: { background: '#0f172a', padding: '1rem', borderRadius: 6, fontSize: '0.75rem', fontFamily: 'monospace', color: '#94a3b8', whiteSpace: 'pre-wrap', maxHeight: 400, overflow: 'auto' },
  branchInput: { padding: '0.3rem 0.5rem', border: '1px solid #334155', borderRadius: 4, background: '#0f172a', color: '#e2e8f0', fontSize: '0.8rem', width: 140, fontFamily: 'monospace' },
  btnSwitch: { padding: '0.3rem 0.7rem', border: 'none', borderRadius: 4, cursor: 'pointer', fontSize: '0.75rem', background: '#1e3a5f', color: '#7dd3fc' },
};

export default App;
