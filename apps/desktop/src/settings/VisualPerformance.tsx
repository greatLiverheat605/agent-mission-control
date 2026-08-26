import type { PerformanceMode } from "../scene/performanceProfile";
import "./visualPerformance.css";
import { useLocale } from "../i18n/LocaleProvider";

const MODES: PerformanceMode[] = ["high", "adaptive", "low"];

export function VisualPerformance({
  value,
  onChange,
  fallback,
  reducedMotion,
}: {
  value: PerformanceMode;
  onChange: (mode: PerformanceMode) => void;
  fallback: boolean;
  reducedMotion: boolean;
}) {
  const { t } = useLocale();
  return <fieldset className="visual-performance">
    <legend>{t("systems.renderQuality")}</legend>
    <div className="visual-performance__segments">
      {MODES.map((mode) => <label key={mode}>
        <input type="radio" name="visual-performance" value={mode} checked={value === mode} onChange={() => onChange(mode)} />
        <span>{t(`systems.${mode}`)}</span>
      </label>)}
    </div>
    {(fallback || reducedMotion) && <output>{fallback ? t("systems.fallback") : t("systems.reducedMotion")}</output>}
  </fieldset>;
}
