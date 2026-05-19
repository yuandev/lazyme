import { StrictMode, useEffect } from 'react'
import { createRoot } from 'react-dom/client'
import App from './App.tsx'
import { initPerformanceMonitor, logMetrics } from './performance.ts'

initPerformanceMonitor();

const PerfDevTools = () => {
  useEffect(() => {
    (window as any).logPerf = logMetrics;
    console.log('%c📊 Performance Monitor initialized!', 'font-size: 14px; font-weight: bold; color: #3b82f6;');
    console.log('%cType `logPerf()` in console to view metrics', 'color: #6b7280;');
  }, []);
  return null;
};

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
    <PerfDevTools />
  </StrictMode>,
)
