import { useTranslation } from "react-i18next";
import { api } from "../../../api";
import { useFeedback } from "../../../app/Feedback";

function sourceUrl(source: string): string | null {
  const value = source.trim();
  if (/^https?:\/\//i.test(value)) return value.replace(/\.git$/i, "");
  const githubSsh = value.match(/^(?:git@github\.com:|ssh:\/\/git@github\.com\/)(.+?)(?:\.git)?$/i);
  if (githubSsh) return `https://github.com/${githubSsh[1]}`;
  if (/^[^/\s]+\/[^/\s]+(?:@[^/\s]+)?$/.test(value)) {
    const [repository, reference] = value.split("@", 2);
    return `https://github.com/${repository}${reference ? `/tree/${reference}` : ""}`;
  }
  return null;
}

export default function SourceLink({ source }: { source: string }) {
  const feedback = useFeedback();
  const { t } = useTranslation("plugins");
  const url = sourceUrl(source);
  const label = source.replace(/^https?:\/\/github\.com\//i, "").replace(/\.git$/i, "");
  if (!url) return <span className="mono muted meta-xs break-all">{source}</span>;
  const isGithub = /github\.com/i.test(url);
  return (
    <button
      type="button"
      className="inline-flex min-w-0 items-center gap-1 text-accent hover:underline"
      title={t("source.openTitle", { url })}
      aria-label={isGithub ? t("source.openGithub") : t("source.openSource")}
      onClick={(event) => {
        event.stopPropagation();
        void api.openUrl(url).catch((error) => feedback.error(String(error)));
      }}
    >
      <span className="mono meta-xs break-all">{label}</span>
    </button>
  );
}
