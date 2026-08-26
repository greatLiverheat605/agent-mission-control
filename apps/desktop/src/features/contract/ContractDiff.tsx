import { useLocale } from "../../i18n/LocaleProvider";

export function ContractDiff({ version, previousVersion = Math.max(1, version - 1) }: { version: number; previousVersion?: number }) {
  const { t } = useLocale();
  return <details className="contract-diff">
    <summary>{t("panel.versionChanges")}</summary>
    <dl>
      <div><dt>{t("panel.previous")}</dt><dd>v{previousVersion}</dd></div>
      <div><dt>{t("panel.current")}</dt><dd>v{version}</dd></div>
    </dl>
  </details>;
}
