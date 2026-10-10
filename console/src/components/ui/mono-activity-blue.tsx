// Ported from Monocharts by Syed Subhan Uddin (MIT), https://github.com/Subhan-code/Monocharts
// See ./mono/LICENSE-monocharts.txt. The activity heatmap with the blue accent.
import { MonoActivityHeatmap, type MonoActivityHeatmapProps } from './mono-activity-heatmap';

export type ActivityBlueProps = Omit<MonoActivityHeatmapProps, 'accentColor'>;

export function ActivityBlue(props: ActivityBlueProps) {
  return <MonoActivityHeatmap {...props} accentColor="blue" />;
}
