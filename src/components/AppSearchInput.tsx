import { Search } from "lucide-react";
import type { RefObject } from "react";

export function AppSearchInput({ value, onChange, placeholder, className = "w-44", inputRef }: {
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
  className?: string;
  inputRef?: RefObject<HTMLInputElement | null>;
}) {
  return <div className={`relative shrink-0 ${className}`}>
    <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-(--text-secondary)" strokeWidth={2} aria-hidden="true" />
    <input ref={inputRef} type="search" className="app-input app-input--pill" value={value}
      placeholder={placeholder} aria-label={placeholder}
      onChange={(event) => onChange(event.target.value)} />
  </div>;
}
