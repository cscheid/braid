import { Component } from "react";
import type { ReactNode, ErrorInfo } from "react";

interface Props {
  children: ReactNode;
}

interface State {
  error: Error | null;
}

export class ErrorBoundary extends Component<Props, State> {
  constructor(props: Props) {
    super(props);
    this.state = { error: null };
  }

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("braid ui render error:", error, info);
  }

  render() {
    if (this.state.error) {
      return (
        <div className="splash splash--error">
          <div className="splash__logo">braid</div>
          <div className="splash__error">
            <strong>Rendering error</strong>
            <br />
            {this.state.error.message}
          </div>
          <pre className="splash__trace">
            {this.state.error.stack}
          </pre>
          <button
            className="btn btn--primary"
            onClick={() => window.location.reload()}
          >
            Reload
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
