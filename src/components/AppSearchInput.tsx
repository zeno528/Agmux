import { Search, X } from "lucide-react";
import type { RefObject } from "react";
import { useTranslation } from "react-i18next";

export function AppSearchInput({ value, onChange, placeholder, className = "w-44", inputRef }: {
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
  className?: string;
  inputRef?: RefObject<HTMLInputElement | null>;
}) {
  const { t } = useTranslation();
  return <div className={`relative shrink-0 ${className}`}>
    <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-(--text-secondary)" strokeWidth={2} aria-hidden="true" />
    <input
      ref={inputRef}
      type="search"
      className={`app-input app-input--pill ${value ? "app-input--pill--clearable" : ""}`}
      value={value}
      placeholder={placeholder}
      aria-label={placeholder}
      onChange={(event) => onChange(event.target.value)}
    />
    {value ? (
      <button
        type="button"
        className="app-search-clear"
        aria-label={t("search.clear")}
        title={t("search.clear")}
        // 不让点击抢走输入框焦点，清空后可继续输入
        onMouseDown={(event) => event.preventDefault()}
        onClick={() => onChange("")}
      >
        <X className="h-3.5 w-3.5" strokeWidth={2} aria-hidden="true" />
      </button>
    ) : null}
  </div>;
}
