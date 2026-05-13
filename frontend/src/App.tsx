import { useState, useEffect, useCallback } from 'react';
import { fetchStatus, fetchCommits, fetchHistory, deployLatest, rollback } from './api';
import type { StatusResponse, CommitInfo, DeployRecord } from './api';

function App() {
  const [tab, setTab] = useState<'status' | 'commits' | 'history'>('status');
  const [status, setStatus] = useState<StatusResponse | null>(null);
  const [commits, setCommits] = useState<CommitInfo[]>([]);
  const [history, setHistory] = useState<DeployRecord[]>([]);
  const [loading, setLoading] = useState(false);

  const refresh = useCallback(async () => {
    const [s, c, h] = await Promise.all([
      fetchStatus(),
      fetchCommits(),
      fetchHistory(),
    ]);
    setStatus(s);
    setCommits(c);
    setHistory(h);
  }, []);

  useEffect(() => {
    refresh();
    const timer = setInterval(refresh, 10_000);
    return () => clearInterval(timer);
  }, [refresh]);

  const handleDeploy = async () => {
    setLoading(true);
    await deployLatest();
    await refresh();
    setLoading(false);
  };

  const handleRollback = async (commit: string) => {
    setLoading(true);
    await rollback(commit);
    await refresh();
    setLoading(false);
  };

  return (
    <div style={s.container}>
      <header style={s.header}>
        <h1 style={s.title}>Deployd</h1>
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
      </header>

      <main style={s.main}>
        {tab === 'status' && status && (
          <StatusPanel status={status} onDeploy={handleDeploy} loading={loading} />
        )}
        {tab === 'commits' && (
          <CommitsPanel
            commits={commits}
            deployedHash={status?.deployed?.commit_hash ?? null}
            onRollback={handleRollback}
            loading={loading}
          />
        )}
        {tab === 'history' && <HistoryPanel history={history} />}
      </main>
    </div>
  );
}

function StatusPanel({ status, onDeploy, loading }: { status: StatusResponse; onDeploy: () => void; loading: boolean }) {
  return (
    <div>
      <div style={s.actions}>
        <button onClick={onDeploy} disabled={loading} style={s.btnPrimary}>
          {loading ? 'Deploying...' : 'Deploy Latest'}
        </button>
      </div>
      <div style={s.card}>
        <h2 style={s.cardTitle}>Status</h2>
        <div style={s.grid}>
          <StatusItem label="Deployed" value={status.deployed?.commit_hash?.substring(0, 7) ?? 'none'} />
          <StatusItem label="Local HEAD" value={status.local_commit?.substring(0, 7) ?? '?'} />
          <StatusItem label="Remote HEAD" value={status.remote_commit?.substring(0, 7) ?? '?'} />
          <StatusItem label="Branch" value={status.branch} />
          <StatusItem label="Polling" value={status.polling ? 'active' : 'paused'} mono={false} />
          <StatusItem label="Interval" value={`${status.interval_secs}s`} mono={false} />
        </div>
      </div>
    </div>
  );
}

function StatusItem({ label, value, mono = true }: { label: string; value: string; mono?: boolean }) {
  return (
    <div style={s.statusItem}>
      <span style={s.label}>{label}</span>
      <span style={{ ...s.value, fontFamily: mono ? 'monospace' : undefined }}>
        {value === 'active' ? (
          <span style={s.badgeGreen}>active</span>
        ) : value === 'paused' ? (
          <span style={s.badgeYellow}>paused</span>
        ) : (
          value
        )}
      </span>
    </div>
  );
}

function CommitsPanel({ commits, deployedHash, onRollback, loading }: {
  commits: CommitInfo[];
  deployedHash: string | null;
  onRollback: (hash: string) => void;
  loading: boolean;
}) {
  return (
    <div style={s.card}>
      <h2 style={s.cardTitle}>Recent Commits</h2>
      <div style={s.list}>
        {commits.map((c) => {
          const isDeployed = c.hash === deployedHash;
          return (
            <div key={c.hash} style={s.listItem}>
              <code style={s.hash}>{c.short_hash}</code>
              <span style={s.msg}>{c.message}</span>
              <button
                onClick={() => onRollback(c.hash)}
                disabled={isDeployed || loading}
                style={{ ...s.btnRollback, ...(isDeployed ? s.btnDisabled : {}) }}
              >
                {isDeployed ? 'current' : 'rollback'}
              </button>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function HistoryPanel({ history }: { history: DeployRecord[] }) {
  const reversed = [...history].reverse();
  return (
    <div style={s.card}>
      <h2 style={s.cardTitle}>Deploy History</h2>
      {reversed.length === 0 ? (
        <p style={s.empty}>No deployments yet</p>
      ) : (
        <div style={s.list}>
          {reversed.map((h, i) => (
            <div key={i} style={s.listItem}>
              <code style={s.hash}>{h.commit_hash.substring(0, 7)}</code>
              <span style={s.msg}>{new Date(h.deployed_at).toLocaleString()}</span>
              <span style={h.success ? s.badgeGreen : s.badgeYellow}>
                {h.success ? 'ok' : 'fail'}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

const s: Record<string, React.CSSProperties> = {
  container: { minHeight: '100vh', background: '#0f172a', color: '#e2e8f0', fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif' },
  header: { padding: '1.5rem 2rem', borderBottom: '1px solid #1e293b', display: 'flex', alignItems: 'center', gap: '2rem', flexWrap: 'wrap' },
  title: { fontSize: '1.25rem', color: '#38bdf8', margin: 0 },
  tabs: { display: 'flex', gap: '0.25rem' },
  tab: { padding: '0.4rem 1rem', border: 'none', borderRadius: 6, cursor: 'pointer', fontSize: '0.85rem', background: 'transparent', color: '#94a3b8' },
  tabActive: { background: '#1e293b', color: '#e2e8f0' },
  main: { padding: '1.5rem 2rem', maxWidth: 900 },
  card: { background: '#1e293b', borderRadius: 8, padding: '1.25rem' },
  cardTitle: { fontSize: '0.9rem', color: '#94a3b8', margin: 0, marginBottom: '0.75rem' },
  grid: { display: 'flex', gap: '2rem', flexWrap: 'wrap' as const },
  statusItem: { display: 'flex', flexDirection: 'column' as const, gap: '0.25rem' },
  label: { fontSize: '0.7rem', color: '#64748b', textTransform: 'uppercase' as const },
  value: { fontSize: '0.9rem' },
  list: { display: 'flex', flexDirection: 'column' as const, gap: '0.4rem' },
  listItem: { display: 'flex', alignItems: 'center', gap: '0.75rem', padding: '0.5rem 0.75rem', background: '#0f172a', borderRadius: 6, fontSize: '0.85rem' },
  hash: { color: '#38bdf8', fontFamily: 'monospace' },
  msg: { flex: 1, color: '#94a3b8', whiteSpace: 'nowrap' as const, overflow: 'hidden', textOverflow: 'ellipsis' },
  actions: { marginBottom: '1rem' },
  btnPrimary: { padding: '0.5rem 1.25rem', border: 'none', borderRadius: 6, cursor: 'pointer', fontSize: '0.85rem', fontWeight: 500, background: '#166534', color: '#4ade80' },
  btnRollback: { padding: '0.3rem 0.7rem', border: 'none', borderRadius: 4, cursor: 'pointer', fontSize: '0.8rem', background: '#b91c1c', color: '#fff', flexShrink: 0 },
  btnDisabled: { opacity: 0.4, cursor: 'default' },
  badgeGreen: { display: 'inline-block', padding: '0.15rem 0.5rem', borderRadius: 999, fontSize: '0.7rem', background: '#166534', color: '#4ade80' },
  badgeYellow: { display: 'inline-block', padding: '0.15rem 0.5rem', borderRadius: 999, fontSize: '0.7rem', background: '#713f12', color: '#facc15' },
  empty: { color: '#64748b', padding: '1rem 0' },
};

export default App;
