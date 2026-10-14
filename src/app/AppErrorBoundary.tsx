import i18next from "i18next";
import { Component, type ErrorInfo, type ReactNode } from "react";

interface AppErrorBoundaryProps {
  children: ReactNode;
}

interface AppErrorBoundaryState {
  error: Error | null;
}

export class AppErrorBoundary extends Component<AppErrorBoundaryProps, AppErrorBoundaryState> {
  state: AppErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): AppErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("Agmux 界面渲染失败", error, info.componentStack); // i18n-exempt: 仅写控制台，用户不可见
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <main className="flex h-screen items-center justify-center bg-(--app-bg) p-6">
        <section className="apple-group w-full max-w-lg text-center">
          <h1 className="apple-title">{i18next.t("error.title")}</h1>
          <p className="muted mt-2 text-sm">{i18next.t("error.description")}</p>
          <button type="button" className="apple-action-button app-button--primary mt-5" onClick={() => window.location.reload()}>
            {i18next.t("error.reload")}
          </button>
          <details className="muted mt-4 text-left text-xs">
            <summary className="cursor-pointer">{i18next.t("error.details")}</summary>
            <pre className="mt-2 whitespace-pre-wrap break-words">{this.state.error.message}</pre>
          </details>
        </section>
      </main>
    );
  }
}
