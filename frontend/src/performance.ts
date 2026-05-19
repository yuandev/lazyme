export interface PerformanceMetrics {
  pageLoad: {
    fp: number | null;
    fcp: number | null;
    lcp: number | null;
    domReady: number | null;
    ttfb: number | null;
  };
  webVitals: {
    lcp: number | null;
    fid: number | null;
    cls: number | null;
    inp: number | null;
  };
  apiMetrics: Map<string, { count: number; totalTime: number; slowCount: number; errorCount: number }>;
  interactionMetrics: Map<string, { count: number; totalTime: number; maxTime: number }>;
  memory: {
    used: number | null;
    total: number | null;
    limit: number | null;
  };
}

const SLOW_API_THRESHOLD = 1000;

const metrics: PerformanceMetrics = {
  pageLoad: { fp: null, fcp: null, lcp: null, domReady: null, ttfb: null },
  webVitals: { lcp: null, fid: null, cls: null, inp: null },
  apiMetrics: new Map(),
  interactionMetrics: new Map(),
  memory: { used: null, total: null, limit: null },
};

let lcpObserver: PerformanceObserver | null = null;
let clsObserver: PerformanceObserver | null = null;
let inpObserver: PerformanceObserver | null = null;
let fidObserver: PerformanceObserver | null = null;

const perfNow = () => performance.now();

export function measureStart() {
  return perfNow();
}

export function measureEnd(start: number, name: string) {
  const duration = perfNow() - start;
  recordInteraction(name, duration);
  return duration;
}

export function measureApi<T>(name: string, fn: () => Promise<T>): Promise<T> {
  const start = perfNow();
  return fn()
    .then(result => {
      recordApi(name, perfNow() - start, false);
      return result;
    })
    .catch(err => {
      recordApi(name, perfNow() - start, true);
      throw err;
    });
}

function recordApi(name: string, duration: number, error: boolean) {
  const existing = metrics.apiMetrics.get(name) || { count: 0, totalTime: 0, slowCount: 0, errorCount: 0 };
  existing.count++;
  existing.totalTime += duration;
  if (duration > SLOW_API_THRESHOLD) existing.slowCount++;
  if (error) existing.errorCount++;
  metrics.apiMetrics.set(name, existing);
}

function recordInteraction(name: string, duration: number) {
  const existing = metrics.interactionMetrics.get(name) || { count: 0, totalTime: 0, maxTime: 0 };
  existing.count++;
  existing.totalTime += duration;
  existing.maxTime = Math.max(existing.maxTime, duration);
  metrics.interactionMetrics.set(name, existing);
}

function initPageLoadMetrics() {
  const nav = performance.getEntriesByType('navigation')[0] as PerformanceNavigationTiming;
  if (nav) {
    metrics.pageLoad.ttfb = nav.responseStart;
    metrics.pageLoad.domReady = nav.domContentLoadedEventEnd;
  }

  const paintObserver = new PerformanceObserver((list) => {
    for (const entry of list.getEntries()) {
      if (entry.name === 'first-paint') {
        metrics.pageLoad.fp = entry.startTime;
      } else if (entry.name === 'first-contentful-paint') {
        metrics.pageLoad.fcp = entry.startTime;
      }
    }
  });
  paintObserver.observe({ entryTypes: ['paint'] });
}

function initLCP() {
  lcpObserver = new PerformanceObserver((list) => {
    const entries = list.getEntries();
    const lastEntry = entries[entries.length - 1];
    metrics.webVitals.lcp = lastEntry.startTime;
    metrics.pageLoad.lcp = lastEntry.startTime;
  });
  lcpObserver.observe({ entryTypes: ['largest-contentful-paint'] });
}

function initCLS() {
  let clsValue = 0;
  let sessionValue = 0;
  let sessionEntries: any[] = [];

  clsObserver = new PerformanceObserver((list) => {
    for (const entry of list.getEntries() as any) {
      if (!entry.hadRecentInput) {
        const firstSessionEntry = sessionEntries[0];
        const lastSessionEntry = sessionEntries[sessionEntries.length - 1];
        if (sessionValue && entry.startTime - lastSessionEntry.startTime < 1000 && entry.startTime - firstSessionEntry.startTime < 5000) {
          sessionValue += entry.value;
          sessionEntries.push(entry);
        } else {
          sessionValue = entry.value;
          sessionEntries = [entry];
        }
        if (sessionValue > clsValue) {
          clsValue = sessionValue;
          metrics.webVitals.cls = clsValue;
        }
      }
    }
  });
  clsObserver.observe({ entryTypes: ['layout-shift'] });
}

function initFID() {
  fidObserver = new PerformanceObserver((list) => {
    for (const entry of list.getEntries() as any) {
      const delay = entry.processingStart - entry.startTime;
      metrics.webVitals.fid = Math.min(metrics.webVitals.fid ?? Infinity, delay);
    }
  });
  fidObserver.observe({ entryTypes: ['first-input'] });
}

function initINP() {
  const interactionLatencies: number[] = [];
  inpObserver = new PerformanceObserver((list) => {
    for (const entry of list.getEntries() as any) {
      const duration = entry.duration;
      interactionLatencies.push(duration);
      interactionLatencies.sort((a, b) => b - a);
      const index = Math.floor(interactionLatencies.length * 0.98);
      metrics.webVitals.inp = interactionLatencies[Math.min(index, interactionLatencies.length - 1)];
    }
  });
  try {
    inpObserver.observe({ entryTypes: ['event', 'first-input'] });
  } catch {
    try {
      inpObserver.observe({ entryTypes: ['event'] });
    } catch {}
  }
}

function updateMemoryMetrics() {
  if ((performance as any).memory) {
    const mem = (performance as any).memory;
    metrics.memory.used = mem.usedJSHeapSize;
    metrics.memory.total = mem.totalJSHeapSize;
    metrics.memory.limit = mem.jsHeapSizeLimit;
  }
}

export function getMetrics() {
  updateMemoryMetrics();
  return JSON.parse(JSON.stringify({
    ...metrics,
    apiMetrics: Object.fromEntries(metrics.apiMetrics),
    interactionMetrics: Object.fromEntries(metrics.interactionMetrics),
  }));
}

export function getApiSummary() {
  const result: Record<string, any> = {};
  metrics.apiMetrics.forEach((stat, name) => {
    result[name] = {
      count: stat.count,
      avgTime: Math.round(stat.totalTime / stat.count),
      slowRate: `${Math.round(stat.slowCount / stat.count * 100)}%`,
      errorRate: `${Math.round(stat.errorCount / stat.count * 100)}%`,
    };
  });
  return result;
}

export function getInteractionSummary() {
  const result: Record<string, any> = {};
  metrics.interactionMetrics.forEach((stat, name) => {
    result[name] = {
      count: stat.count,
      avgTime: Math.round(stat.totalTime / stat.count),
      maxTime: Math.round(stat.maxTime),
    };
  });
  return result;
}

export function logMetrics() {
  console.log('%c📊 Performance Metrics', 'font-size: 16px; font-weight: bold; color: #3b82f6;');
  console.log('%c--- Page Load ---', 'font-weight: bold; color: #60a5fa;');
  console.table(metrics.pageLoad);
  console.log('%c--- Web Vitals ---', 'font-weight: bold; color: #60a5fa;');
  console.table(metrics.webVitals);
  console.log('%c--- API Performance ---', 'font-weight: bold; color: #60a5fa;');
  console.table(getApiSummary());
  console.log('%c--- Interaction Performance ---', 'font-weight: bold; color: #60a5fa;');
  console.table(getInteractionSummary());
  if (metrics.memory.used) {
    console.log('%c--- Memory Usage ---', 'font-weight: bold; color: #60a5fa;');
    console.table({
      used: `${(metrics.memory.used / 1024 / 1024).toFixed(2)} MB`,
      total: `${(metrics.memory.total! / 1024 / 1024).toFixed(2)} MB`,
      limit: `${(metrics.memory.limit! / 1024 / 1024).toFixed(2)} MB`,
    });
  }
}

export function initPerformanceMonitor() {
  if (typeof performance === 'undefined') return;
  initPageLoadMetrics();
  initLCP();
  initCLS();
  initFID();
  initINP();
  setInterval(updateMemoryMetrics, 5000);
  if (import.meta.env.DEV) {
    (window as any).perf = { getMetrics, logMetrics, getApiSummary, getInteractionSummary };
  }
}
