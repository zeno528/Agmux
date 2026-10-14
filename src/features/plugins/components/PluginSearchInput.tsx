import { AppSearchInput } from "../../../components/AppSearchInput";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";

export default function PluginSearchInput({ value, onChange }: { value: string; onChange: (value: string) => void }) {
  const { t } = useTranslation("plugins");
  const inputRef = useRef<HTMLInputElement>(null);
  // Ctrl/Cmd+K 聚焦搜索框：window 级监听保证焦点在别处也生效，抢在浏览器站点搜索前 preventDefault
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        inputRef.current?.focus();
        inputRef.current?.select();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
  return <AppSearchInput inputRef={inputRef} value={value} onChange={onChange} placeholder={t("list.searchPlaceholder")} />;
}
