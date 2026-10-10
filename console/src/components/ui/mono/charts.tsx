// Ported from Monocharts by Syed Subhan Uddin (MIT), https://github.com/Subhan-code/Monocharts
// See ./LICENSE-monocharts.txt. Originals: src/components/mono-charts/MonoRounded{Line,Bar,Area,
// Donut,Composed,Scatter}Chart.tsx — made data-driven via props; visuals kept.
import { useId, useState, type ReactNode } from 'react';
import {
  ResponsiveContainer,
  LineChart,
  Line,
  BarChart,
  Bar,
  AreaChart,
  Area,
  PieChart,
  Pie,
  Cell,
  ComposedChart,
  ScatterChart,
  Scatter,
  XAxis,
  YAxis,
  ZAxis,
  CartesianGrid,
  Tooltip,
} from 'recharts';
import { DitherChartTooltipContent } from './tooltip';

/* ── shared ───────────────────────────────────────────────────────────── */

export type MonoTheme = 'dark' | 'light';

export interface MonoCommonProps {
  theme?: MonoTheme;
  compact?: boolean;
  title: string;
  badge: string;
  /** The big number in the header. */
  value: ReactNode;
  /** Small text after the big number. */
  unit: string;
  footerLeft: string;
  footerRight: string;
}

const tickFill = (isDark: boolean) => (isDark ? '#71717A' : '#A1A1AA');

interface ShellProps extends MonoCommonProps {
  /** Header margin: the donut uses mb-1, the rest mb-2. */
  headerClass?: string;
  /** Extra classes for the stage (the donut centres its content). */
  stageClass?: string;
  controls?: ReactNode;
  footer?: ReactNode;
  children: ReactNode;
}

function Shell({
  theme = 'dark',
  compact = false,
  title,
  badge,
  value,
  unit,
  footerLeft,
  footerRight,
  headerClass = 'mb-2',
  stageClass = '',
  controls,
  footer,
  children,
}: ShellProps) {
  const isDark = theme === 'dark';
  return (
    <div
      className={`relative w-full rounded-[24px] transition-all duration-300 group flex flex-col justify-between overflow-hidden p-4 sm:p-5 ${
        compact ? 'h-[220px] sm:h-[268px]' : 'min-h-[290px]'
      } ${
        isDark
          ? 'bg-[#181818] shadow-[inset_0_1px_0_rgba(255,255,255,0.04)] text-white hover:bg-[#202020]'
          : 'bg-white shadow-[0_4px_20px_rgba(0,0,0,0.04)] border border-neutral-100 text-black hover:shadow-[0_6px_24px_rgba(0,0,0,0.06)]'
      }`}
    >
      {/* Header */}
      <div className={`flex items-center justify-between ${headerClass}`}>
        <div>
          <div className="flex items-center gap-2">
            <span className={`text-xs font-semibold tracking-wider uppercase ${isDark ? 'text-neutral-400' : 'text-neutral-500'}`}>
              {title}
            </span>
            <span className={`inline-flex items-center px-1.5 py-0.5 rounded-full text-[10px] font-mono border ${isDark ? 'bg-white/10 text-white border-white/20' : 'bg-black/5 text-black border-black/15'}`}>
              {badge}
            </span>
          </div>
          <div className="text-xl font-bold tracking-tight tabular-nums mt-0.5 font-sans">
            {value} <span className="text-xs font-normal opacity-70">{unit}</span>
          </div>
        </div>
        {controls}
      </div>

      {/* Main Recharts Stage */}
      <div className={`relative w-full flex-1 rounded-[14px] overflow-hidden p-2 transition-colors duration-300 ${stageClass} ${
        isDark ? 'bg-[#131313]' : 'bg-[#f4f4f6]'
      }`}>
        {children}
      </div>

      {footer ?? (
        <div className="flex items-center justify-between mt-3 pt-1 border-t border-white/5 text-[11px] font-mono">
          <span className={isDark ? 'text-neutral-400' : 'text-neutral-600'}>
            {footerLeft}
          </span>
          <span className={isDark ? 'text-white font-medium' : 'text-black font-medium'}>
            {footerRight}
          </span>
        </div>
      )}
    </div>
  );
}

function EmptyStage({ theme = 'dark', compact = false }: { theme?: MonoTheme; compact?: boolean }) {
  const isDark = theme === 'dark';
  return (
    <div
      className={`w-full flex items-center justify-center text-[10px] font-mono ${isDark ? 'text-neutral-500' : 'text-neutral-400'}`}
      style={{ height: compact ? 130 : 160 }}
    >
      no data yet
    </div>
  );
}

/** Pill segmented control used by the line, bar and area charts. */
function Segmented<T extends string>({
  theme,
  options,
  active,
  onChange,
  labelOf,
}: {
  theme: MonoTheme;
  options: readonly T[];
  active: T;
  onChange: (v: T) => void;
  labelOf: (v: T) => string;
}) {
  const isDark = theme === 'dark';
  return (
    <div className={`p-0.5 rounded-full border flex items-center gap-0.5 ${
      isDark ? 'bg-white/5 border-white/10' : 'bg-neutral-100 border-neutral-200'
    }`}>
      {options.map((o) => (
        <button
          key={o}
          type="button"
          onClick={() => onChange(o)}
          className={`border-0 font-[inherit] px-2.5 py-0.5 rounded-full text-[11px] font-medium capitalize transition-all cursor-pointer ${
            active === o
              ? isDark
                ? 'bg-white text-black font-semibold shadow-sm'
                : 'bg-black text-white font-semibold shadow-sm'
              : isDark
              ? 'bg-transparent text-neutral-400 hover:text-white'
              : 'bg-transparent text-neutral-600 hover:text-black'
          }`}
        >
          {labelOf(o)}
        </button>
      ))}
    </div>
  );
}

/* ── MonoLine ("Spline Dynamics") ─────────────────────────────────────── */

export interface MonoLineProps extends MonoCommonProps {
  data: { label: string; value: number; secondary: number }[];
  /** Series names for the tooltip: [main, secondary]. */
  names?: [string, string];
}

export function MonoLine({ data, names = ['Active', 'Baseline'], ...common }: MonoLineProps) {
  const { theme = 'dark', compact = false } = common;
  const isDark = theme === 'dark';
  const [activeSeries, setActiveSeries] = useState<'value' | 'all'>('all');

  return (
    <Shell
      {...common}
      controls={
        <Segmented
          theme={theme}
          options={['all', 'value'] as const}
          active={activeSeries}
          onChange={setActiveSeries}
          labelOf={(s) => (s === 'all' ? 'Dual' : 'Single')}
        />
      }
    >
      {data.length === 0 ? (
        <EmptyStage theme={theme} compact={compact} />
      ) : (
        <ResponsiveContainer width="100%" height={compact ? 130 : 160}>
          <LineChart data={data} margin={{ top: 12, right: 12, left: -22, bottom: 0 }}>
            <CartesianGrid
              strokeDasharray="3 3"
              vertical={false}
              stroke={isDark ? 'rgba(255,255,255,0.05)' : 'rgba(0,0,0,0.05)'}
            />
            <XAxis dataKey="label" tickLine={false} axisLine={false} tick={{ fontSize: 10, fill: tickFill(isDark) }} />
            <YAxis tickLine={false} axisLine={false} tick={{ fontSize: 10, fill: tickFill(isDark) }} />
            <Tooltip content={<DitherChartTooltipContent theme={theme} indicator="dot" />} />

            {activeSeries === 'all' && (
              <Line
                type="monotone"
                dataKey="secondary"
                name={names[1]}
                stroke={isDark ? '#52525B' : '#A1A1AA'}
                strokeWidth={2}
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeDasharray="4 4"
                dot={false}
                animationDuration={900}
              />
            )}

            <Line
              type="monotone"
              dataKey="value"
              name={names[0]}
              stroke={isDark ? '#FFFFFF' : '#09090B'}
              strokeWidth={3}
              strokeLinecap="round"
              strokeLinejoin="round"
              dot={{
                r: 4,
                fill: isDark ? '#FFFFFF' : '#09090B',
                stroke: isDark ? '#181818' : '#FFFFFF',
                strokeWidth: 2,
              }}
              activeDot={{
                r: 6,
                fill: isDark ? '#FFFFFF' : '#09090B',
                stroke: isDark ? '#A1A1AA' : '#52525B',
                strokeWidth: 2,
              }}
              animationDuration={800}
            />
          </LineChart>
        </ResponsiveContainer>
      )}
    </Shell>
  );
}

/* ── MonoBars ("Mono Pill Pillars") ───────────────────────────────────── */

export interface MonoBarsProps extends MonoCommonProps {
  data: { label: string; value: number; secondary: number }[];
  /** Series names for the tooltip: [main, secondary]. */
  names?: [string, string];
}

export function MonoBars({ data, names = ['Primary Output', 'Secondary Output'], ...common }: MonoBarsProps) {
  const { theme = 'dark', compact = false } = common;
  const isDark = theme === 'dark';
  const [layout, setLayout] = useState<'vertical' | 'horizontal'>('vertical');
  const isHorizontal = layout === 'horizontal';

  return (
    <Shell
      {...common}
      controls={
        <Segmented
          theme={theme}
          options={['vertical', 'horizontal'] as const}
          active={layout}
          onChange={setLayout}
          labelOf={(l) => (l === 'vertical' ? 'Col' : 'Row')}
        />
      }
    >
      {data.length === 0 ? (
        <EmptyStage theme={theme} compact={compact} />
      ) : (
        <ResponsiveContainer width="100%" height={compact ? 130 : 160}>
          <BarChart
            data={data}
            layout={isHorizontal ? 'vertical' : 'horizontal'}
            margin={{ top: 12, right: 12, left: isHorizontal ? 0 : -22, bottom: 0 }}
          >
            <CartesianGrid
              strokeDasharray="2 2"
              vertical={false}
              stroke={isDark ? 'rgba(255,255,255,0.05)' : 'rgba(0,0,0,0.05)'}
            />
            {isHorizontal ? (
              <>
                <XAxis type="number" hide />
                <YAxis dataKey="label" type="category" tickLine={false} axisLine={false} tick={{ fontSize: 10, fill: tickFill(isDark) }} />
              </>
            ) : (
              <>
                <XAxis dataKey="label" tickLine={false} axisLine={false} tick={{ fontSize: 10, fill: tickFill(isDark) }} />
                <YAxis tickLine={false} axisLine={false} tick={{ fontSize: 10, fill: tickFill(isDark) }} />
              </>
            )}
            <Tooltip content={<DitherChartTooltipContent theme={theme} indicator="dot" />} />

            {/* Primary Rounded Pill Column */}
            <Bar
              dataKey="value"
              name={names[0]}
              fill={isDark ? '#FFFFFF' : '#09090B'}
              radius={isHorizontal ? [0, 8, 8, 0] : [8, 8, 8, 8]}
              barSize={isHorizontal ? 12 : 16}
              animationDuration={800}
            />

            {/* Muted Secondary Column */}
            <Bar
              dataKey="secondary"
              name={names[1]}
              fill={isDark ? 'rgba(255,255,255,0.2)' : 'rgba(0,0,0,0.15)'}
              radius={isHorizontal ? [0, 8, 8, 0] : [8, 8, 8, 8]}
              barSize={isHorizontal ? 12 : 16}
              animationDuration={1000}
            />
          </BarChart>
        </ResponsiveContainer>
      )}
    </Shell>
  );
}

/* ── MonoArea ("Mono Curved Wave") ────────────────────────────────────── */

export interface MonoAreaProps extends MonoCommonProps {
  data: { label: string; value: number }[];
  /** Series name for the tooltip. */
  name?: string;
}

export function MonoArea({ data, name = 'Volume Flow', ...common }: MonoAreaProps) {
  const { theme = 'dark', compact = false } = common;
  const isDark = theme === 'dark';
  const idPrefix = useId().replace(/:/g, '');
  const [curve, setCurve] = useState<'monotone' | 'natural'>('monotone');

  return (
    <Shell
      {...common}
      controls={
        <Segmented
          theme={theme}
          options={['monotone', 'natural'] as const}
          active={curve}
          onChange={setCurve}
          labelOf={(c) => c}
        />
      }
    >
      {data.length === 0 ? (
        <EmptyStage theme={theme} compact={compact} />
      ) : (
        <>
          <svg className="absolute w-0 h-0 pointer-events-none">
            <defs>
              <linearGradient id={`${idPrefix}mono-area-gradient`} x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor={isDark ? '#FFFFFF' : '#09090B'} stopOpacity={isDark ? '0.35' : '0.25'} />
                <stop offset="100%" stopColor={isDark ? '#FFFFFF' : '#09090B'} stopOpacity="0.0" />
              </linearGradient>
            </defs>
          </svg>

          <ResponsiveContainer width="100%" height={compact ? 130 : 160}>
            <AreaChart data={data} margin={{ top: 12, right: 12, left: -22, bottom: 0 }}>
              <CartesianGrid
                strokeDasharray="2 2"
                vertical={false}
                stroke={isDark ? 'rgba(255,255,255,0.05)' : 'rgba(0,0,0,0.05)'}
              />
              <XAxis dataKey="label" tickLine={false} axisLine={false} tick={{ fontSize: 10, fill: tickFill(isDark) }} />
              <YAxis tickLine={false} axisLine={false} tick={{ fontSize: 10, fill: tickFill(isDark) }} />
              <Tooltip content={<DitherChartTooltipContent theme={theme} indicator="dot" />} />

              {/* Mono Rounded Area Fill */}
              <Area
                type={curve}
                dataKey="value"
                name={name}
                stroke={isDark ? '#FFFFFF' : '#09090B'}
                strokeWidth={2.5}
                strokeLinecap="round"
                strokeLinejoin="round"
                fill={`url(#${idPrefix}mono-area-gradient)`}
                animationDuration={900}
              />
            </AreaChart>
          </ResponsiveContainer>
        </>
      )}
    </Shell>
  );
}

/* ── MonoDonut ("Mono Rounded Donut") ─────────────────────────────────── */

export interface MonoDonutProps extends MonoCommonProps {
  /** Raw counts; percentages are computed internally. */
  data: { name: string; value: number }[];
  /** Small text under the total in the donut centre. */
  centerLabel?: string;
}

const formatPct = (pct: number) =>
  `${pct >= 10 || pct === 0 ? Math.round(pct) : Math.round(pct * 10) / 10}%`;

export function MonoDonut({ data, centerLabel = 'total', ...common }: MonoDonutProps) {
  const { theme = 'dark', compact = false } = common;
  const isDark = theme === 'dark';
  const [hoverIndex, setHoverIndex] = useState<number | null>(null);

  const total = data.reduce((acc, item) => acc + item.value, 0);
  const pctOf = (v: number) => (total > 0 ? (v / total) * 100 : 0);
  const hovered = hoverIndex !== null ? data[hoverIndex] : undefined;
  const isEmpty = data.length === 0 || total <= 0;

  return (
    <Shell
      {...common}
      headerClass="mb-1"
      stageClass="flex items-center justify-center"
      footer={
        <div className="flex items-center justify-around mt-3 pt-1 border-t border-white/5 text-[10px]">
          {data.map((seg, idx) => (
            <div key={idx} className="flex items-center gap-1">
              <span className={`w-1.5 h-1.5 rounded-full ${isDark ? 'bg-white/70' : 'bg-black/60'}`} />
              <span className={isDark ? 'text-neutral-400' : 'text-neutral-600'}>
                {seg.name} {isEmpty ? '' : formatPct(pctOf(seg.value))}
              </span>
            </div>
          ))}
        </div>
      }
    >
      {isEmpty ? (
        <EmptyStage theme={theme} compact={compact} />
      ) : (
        <>
          <ResponsiveContainer width="100%" height={compact ? 130 : 160}>
            <PieChart>
              <Tooltip content={<DitherChartTooltipContent theme={theme} indicator="dot" />} />
              <Pie
                data={data}
                dataKey="value"
                nameKey="name"
                cx="50%"
                cy="50%"
                innerRadius={compact ? 38 : 46}
                outerRadius={compact ? 58 : 68}
                paddingAngle={6}
                cornerRadius={8}
                strokeLinecap="round"
                onMouseEnter={(_, idx) => setHoverIndex(idx)}
                onMouseLeave={() => setHoverIndex(null)}
                animationDuration={900}
              >
                {data.map((_, index) => {
                  const isHovered = hoverIndex === index;
                  const fillColor = isDark
                    ? index === 0
                      ? '#FFFFFF'
                      : index === 1
                      ? 'rgba(255,255,255,0.7)'
                      : index === 2
                      ? 'rgba(255,255,255,0.4)'
                      : 'rgba(255,255,255,0.2)'
                    : index === 0
                    ? '#09090B'
                    : index === 1
                    ? 'rgba(9,9,11,0.7)'
                    : index === 2
                    ? 'rgba(9,9,11,0.4)'
                    : 'rgba(9,9,11,0.2)';

                  return (
                    <Cell
                      key={`mono-cell-${index}`}
                      fill={fillColor}
                      stroke={isDark ? '#181818' : '#FFFFFF'}
                      strokeWidth={2}
                      style={{
                        transform: isHovered ? 'scale(1.05)' : 'scale(1)',
                        transformOrigin: 'center center',
                        transition: 'transform 0.2s cubic-bezier(0.16, 1, 0.3, 1)',
                        cursor: 'pointer',
                      }}
                    />
                  );
                })}
              </Pie>
            </PieChart>
          </ResponsiveContainer>

          {/* Center Stat Callout */}
          <div className="absolute inset-0 flex flex-col items-center justify-center pointer-events-none">
            <span className="text-sm font-bold tabular-nums font-sans">
              {hovered ? formatPct(pctOf(hovered.value)) : total.toLocaleString()}
            </span>
            <span className={`text-[10px] ${isDark ? 'text-neutral-400' : 'text-neutral-500'}`}>
              {hovered ? hovered.name : centerLabel}
            </span>
          </div>
        </>
      )}
    </Shell>
  );
}

/* ── MonoComposed ("Mono Hybrid Spline") ──────────────────────────────── */

export interface MonoComposedProps extends MonoCommonProps {
  data: { label: string; bar: number; line: number }[];
  /** Series names for the tooltip: [bar, line]. */
  names?: [string, string];
}

export function MonoComposed({ data, names = ['Count', 'Trend'], ...common }: MonoComposedProps) {
  const { theme = 'dark', compact = false } = common;
  const isDark = theme === 'dark';
  const [showLine, setShowLine] = useState<boolean>(true);

  return (
    <Shell
      {...common}
      controls={
        <button
          type="button"
          onClick={() => setShowLine(!showLine)}
          className={`font-[inherit] px-2.5 py-1 rounded-full text-[11px] font-medium border transition-all cursor-pointer ${
            showLine
              ? isDark
                ? 'bg-white/10 text-white border-white/20'
                : 'bg-neutral-100 text-black border-neutral-300'
              : isDark
              ? 'bg-transparent border-white/5 text-neutral-500'
              : 'bg-transparent border-neutral-200 text-neutral-400'
          }`}
        >
          {showLine ? 'Spline On' : 'Spline Off'}
        </button>
      }
    >
      {data.length === 0 ? (
        <EmptyStage theme={theme} compact={compact} />
      ) : (
        <ResponsiveContainer width="100%" height={compact ? 130 : 160}>
          <ComposedChart data={data} margin={{ top: 12, right: 12, left: -22, bottom: 0 }}>
            <CartesianGrid
              strokeDasharray="2 2"
              vertical={false}
              stroke={isDark ? 'rgba(255,255,255,0.05)' : 'rgba(0,0,0,0.05)'}
            />
            <XAxis dataKey="label" tickLine={false} axisLine={false} tick={{ fontSize: 10, fill: tickFill(isDark) }} />
            <YAxis tickLine={false} axisLine={false} tick={{ fontSize: 10, fill: tickFill(isDark) }} />
            <Tooltip content={<DitherChartTooltipContent theme={theme} indicator="dot" />} />

            {/* Rounded Pill Bar Column */}
            <Bar
              dataKey="bar"
              name={names[0]}
              fill={isDark ? 'rgba(255,255,255,0.18)' : 'rgba(9,9,11,0.12)'}
              stroke={isDark ? 'rgba(255,255,255,0.4)' : 'rgba(9,9,11,0.3)'}
              strokeWidth={1}
              radius={[8, 8, 8, 8]}
              barSize={20}
              animationDuration={800}
            />

            {/* Smooth Spline Overlay */}
            {showLine && (
              <Line
                type="monotone"
                dataKey="line"
                name={names[1]}
                stroke={isDark ? '#FFFFFF' : '#09090B'}
                strokeWidth={3}
                strokeLinecap="round"
                strokeLinejoin="round"
                dot={{
                  r: 4,
                  fill: isDark ? '#FFFFFF' : '#09090B',
                  stroke: isDark ? '#181818' : '#FFFFFF',
                  strokeWidth: 2,
                }}
                animationDuration={900}
              />
            )}
          </ComposedChart>
        </ResponsiveContainer>
      )}
    </Shell>
  );
}

/* ── MonoScatter ("Mono Scatter Matrix") ──────────────────────────────── */

export interface MonoScatterProps extends MonoCommonProps {
  data: { x: number; y: number; z?: number; label?: string }[];
  xName?: string;
  yName?: string;
}

export function MonoScatter({ data, xName = 'x', yName = 'y', ...common }: MonoScatterProps) {
  const { theme = 'dark', compact = false } = common;
  const isDark = theme === 'dark';

  // Only weight node size when the caller gave sizes; points without z get the smallest one.
  const zs = data.flatMap((d) => (typeof d.z === 'number' ? [d.z] : []));
  const hasZ = zs.length > 0;
  const minZ = hasZ ? Math.min(...zs) : 0;
  const points = hasZ ? data.map((d) => ({ ...d, z: d.z ?? minZ })) : data;

  return (
    <Shell {...common}>
      {data.length === 0 ? (
        <EmptyStage theme={theme} compact={compact} />
      ) : (
        <ResponsiveContainer width="100%" height={compact ? 130 : 160}>
          <ScatterChart margin={{ top: 12, right: 12, left: -22, bottom: 0 }}>
            <CartesianGrid
              strokeDasharray="2 2"
              stroke={isDark ? 'rgba(255,255,255,0.05)' : 'rgba(0,0,0,0.05)'}
            />
            <XAxis dataKey="x" name={xName} type="number" tickLine={false} axisLine={false} tick={{ fontSize: 10, fill: tickFill(isDark) }} />
            <YAxis dataKey="y" name={yName} type="number" tickLine={false} axisLine={false} tick={{ fontSize: 10, fill: tickFill(isDark) }} />
            {hasZ && <ZAxis dataKey="z" range={[60, 240]} />}
            <Tooltip
              content={(props) => (
                <DitherChartTooltipContent
                  active={props.active}
                  payload={props.payload as any[] | undefined}
                  label={(props.payload?.[0]?.payload as { label?: string } | undefined)?.label}
                  theme={theme}
                  indicator="dot"
                />
              )}
              cursor={{ strokeDasharray: '3 3' }}
            />

            <Scatter
              name="Nodes"
              data={points}
              fill={isDark ? '#FFFFFF' : '#09090B'}
              stroke={isDark ? 'rgba(255,255,255,0.5)' : 'rgba(9,9,11,0.5)'}
              strokeWidth={1.5}
              animationDuration={800}
            />
          </ScatterChart>
        </ResponsiveContainer>
      )}
    </Shell>
  );
}
