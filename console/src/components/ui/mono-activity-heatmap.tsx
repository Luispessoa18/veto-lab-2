// Ported from Monocharts by Syed Subhan Uddin (MIT), https://github.com/Subhan-code/Monocharts
// See ./mono/LICENSE-monocharts.txt. Originally src/components/mono-charts/MonoActivityHeatmap.tsx;
// now accepts real contributions and copy via props (demo data when `contributions` is omitted).
import { useState, useMemo } from 'react';
import { motion } from 'motion/react';
import { cn } from '@/lib/utils';

export type ContributionLevel = 0 | 1 | 2 | 3 | 4;

export type Contribution = {
  date: string;
  count: number;
  level: ContributionLevel;
};

const MONTH_NAMES = [
  'Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun',
  'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'
];

export interface MonoActivityHeatmapProps {
  theme?: 'dark' | 'light';
  accentColor?: 'green' | 'blue' | 'purple' | 'mono';
  compact?: boolean;
  /** Days in order, oldest first (grouped 7 per column). Omit for random demo data. */
  contributions?: Contribution[];
  title?: string;
  /** Defaults to the accent's name, e.g. "Sky Blue Grid". */
  badge?: string;
  unit?: string;
  footerLeft?: string;
  footerRight?: string;
  /** Singular/plural for the hover text, e.g. ['item', 'items']. */
  itemLabel?: [string, string];
}

function generateDemoContributions(weeks: number): Contribution[] {
  const today = new Date();
  return Array.from({ length: weeks * 7 }, (_, i) => {
    const date = new Date(today);
    date.setDate(date.getDate() - (weeks * 7 - 1 - i));

    const rand = Math.random();
    let level: ContributionLevel = 0;
    let count = 0;

    if (rand > 0.35) {
      level = Math.floor(Math.random() * 4 + 1) as ContributionLevel;
      count = level * 3 + Math.floor(Math.random() * 4);
    }

    return {
      date: date.toISOString().slice(0, 10),
      count,
      level,
    };
  });
}

function toWeeks(contributions: Contribution[]) {
  const weeks: Contribution[][] = [];
  for (let i = 0; i < contributions.length; i += 7) {
    weeks.push(contributions.slice(i, i + 7));
  }
  return weeks;
}

/** Month index of a date string; reads YYYY-MM-DD directly to avoid timezone shifts. */
function monthOf(date: string): number | null {
  const m = /^\d{4}-(\d{2})/.exec(date);
  if (m) {
    const idx = Number(m[1]) - 1;
    return idx >= 0 && idx < 12 ? idx : null;
  }
  const d = new Date(date);
  return Number.isNaN(d.getTime()) ? null : d.getMonth();
}

/** One label slot per week column: the month name where a new month starts, else ''. */
function monthLabels(weeks: Contribution[][]): string[] {
  let prev: number | null = null;
  let lastLabelAt = -Infinity;
  return weeks.map((week, wIdx) => {
    const month = week.length ? monthOf(week[0].date) : null;
    let label = '';
    // Keep labels at least 2 columns apart so narrow columns don't collide.
    if (month !== null && month !== prev && wIdx - lastLabelAt >= 2) {
      label = MONTH_NAMES[month];
      lastLabelAt = wIdx;
    }
    if (month !== null) prev = month;
    return label;
  });
}

export function MonoActivityHeatmap({
  theme = 'dark',
  accentColor = 'green',
  compact = false,
  contributions,
  title = 'Activity Heatmap',
  badge,
  unit = 'contributions',
  footerLeft,
  footerRight,
  itemLabel = ['item', 'items'],
}: MonoActivityHeatmapProps) {
  const isDark = theme === 'dark';
  const [hoveredDay, setHoveredDay] = useState<Contribution | null>(null);

  const demoData = useMemo(() => generateDemoContributions(20), []);
  const days = contributions ?? demoData;
  const weeks = useMemo(() => toWeeks(days), [days]);
  const months = useMemo(() => monthLabels(weeks), [weeks]);

  const totalContributions = useMemo(
    () => days.reduce((sum, d) => sum + d.count, 0),
    [days]
  );

  // Color config according to accentColor prop
  const colorScale = useMemo(() => {
    switch (accentColor) {
      case 'green':
        return {
          bg: '#39d353',
          badgeClass: 'bg-emerald-500/20 text-emerald-400 border-emerald-500/30',
          badgeText: 'Emerald Matrix',
        };
      case 'blue':
        return {
          bg: '#3b82f6',
          badgeClass: 'bg-blue-500/20 text-blue-400 border-blue-500/30',
          badgeText: 'Sky Blue Grid',
        };
      case 'purple':
        return {
          bg: '#a855f7',
          badgeClass: 'bg-purple-500/20 text-purple-400 border-purple-500/30',
          badgeText: 'Violet Pulse',
        };
      case 'mono':
      default:
        return {
          bg: isDark ? '#FFFFFF' : '#09090B',
          badgeClass: 'bg-white/10 text-white border-white/20',
          badgeText: 'Monochrome Heat',
        };
    }
  }, [accentColor, isDark]);

  const opacityForLevel = (lvl: ContributionLevel) => {
    switch (lvl) {
      case 0: return isDark ? 0.06 : 0.08;
      case 1: return 0.3;
      case 2: return 0.55;
      case 3: return 0.8;
      case 4: return 1;
      default: return isDark ? 0.06 : 0.08;
    }
  };

  return (
    <div
      className={cn(
        'relative w-full rounded-[24px] transition-all duration-300 group flex flex-col justify-between overflow-hidden p-4 sm:p-5 font-sans',
        compact ? 'h-[220px] sm:h-[268px]' : 'min-h-[290px]',
        isDark
          ? 'bg-[#181818] shadow-[inset_0_1px_0_rgba(255,255,255,0.04)] text-white hover:bg-[#202020]'
          : 'bg-white shadow-[0_4px_20px_rgba(0,0,0,0.04)] border border-neutral-100 text-black hover:shadow-[0_6px_24px_rgba(0,0,0,0.06)]'
      )}
    >
      {/* Header */}
      <div className="flex items-center justify-between mb-2">
        <div>
          <div className="flex items-center gap-2">
            <span className={`text-xs font-semibold tracking-wider uppercase ${isDark ? 'text-neutral-400' : 'text-neutral-500'}`}>
              {title}
            </span>
            <span className={cn('inline-flex items-center px-1.5 py-0.5 rounded-full text-[10px] font-mono border', colorScale.badgeClass)}>
              {badge ?? colorScale.badgeText}
            </span>
          </div>
          <div className="text-xl font-bold tracking-tight tabular-nums mt-0.5 font-sans">
            {totalContributions.toLocaleString()} <span className="text-xs font-normal opacity-70">{unit}</span>
          </div>
        </div>
      </div>

      {/* Main Heatmap Stage Grid */}
      <div className={cn(
        'relative w-full flex-1 rounded-[14px] overflow-hidden p-3 transition-colors duration-300 flex flex-col justify-center items-center',
        isDark ? 'bg-[#131313]' : 'bg-[#f4f4f6]'
      )}>
        {weeks.length === 0 ? (
          <span className={`text-[10px] font-mono ${isDark ? 'text-neutral-500' : 'text-neutral-400'}`}>
            no data yet
          </span>
        ) : (
          <>
            {/* Month Headers: one slot per week column, labelled where a month starts */}
            <div className="flex justify-center gap-1.5 mb-1.5 w-full">
              {months.map((m, idx) => (
                <span key={idx} className={`text-[10px] font-mono flex-1 min-w-0 text-left whitespace-nowrap overflow-visible ${isDark ? 'text-neutral-400' : 'text-neutral-500'}`}>
                  {m || ' '}
                </span>
              ))}
            </div>

            {/* Week Heatmap Grid */}
            <div className="flex justify-center gap-1.5 w-full overflow-hidden" onMouseLeave={() => setHoveredDay(null)}>
              {weeks.map((week, wIdx) => (
                <div key={wIdx} className="flex flex-col gap-1.5 flex-1 items-center">
                  {week.map((day, dIdx) => (
                    <motion.div
                      key={`${wIdx}-${dIdx}`}
                      onMouseEnter={() => setHoveredDay(day)}
                      className="w-full h-3 sm:h-3.5 rounded-[3px] transition-all cursor-pointer hover:scale-125"
                      style={{
                        backgroundColor: colorScale.bg,
                        opacity: opacityForLevel(day.level),
                      }}
                      whileHover={{ scale: 1.25 }}
                    />
                  ))}
                </div>
              ))}
            </div>

            {/* Active Cell Tooltip Callout */}
            <div className="h-5 mt-2 flex items-center justify-center">
              {hoveredDay ? (
                <span className={`text-[10px] font-mono ${isDark ? 'text-neutral-300' : 'text-neutral-700'}`}>
                  {hoveredDay.count} {hoveredDay.count === 1 ? itemLabel[0] : itemLabel[1]} on {hoveredDay.date}
                </span>
              ) : (
                <span className={`text-[10px] font-mono ${isDark ? 'text-neutral-400' : 'text-neutral-500'}`}>
                  Hover tiles for metrics
                </span>
              )}
            </div>
          </>
        )}
      </div>

      {/* Footer Details */}
      <div className="flex items-center justify-between mt-3 pt-1 border-t border-white/5 text-[11px] font-mono">
        <span className={isDark ? 'text-neutral-400' : 'text-neutral-600'}>
          {footerLeft ?? `${weeks.length} Weeks x 7 Days Grid`}
        </span>
        <span className={isDark ? 'text-white font-medium' : 'text-black font-medium'}>
          {footerRight ?? `Last ${days.length} days`}
        </span>
      </div>
    </div>
  );
}
