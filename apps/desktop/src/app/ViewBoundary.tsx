import { Component, type ReactNode } from "react";
import { ViewError } from "../design/ViewError";

interface Props {
  children: ReactNode;
  /** Records the failure through the interceptor; returns the words to show. */
  onCrash: () => string;
}

interface State {
  failed: boolean;
  text: string | null;
}

/**
 * Around each view: a view that fails to paint is the generic
 * failure, reported like any other, instead of a blank screen. Mount it with
 * the view as its `key`, so changing view gives the new one a fresh chance.
 */
export class ViewBoundary extends Component<Props, State> {
  state: State = { failed: false, text: null };

  static getDerivedStateFromError(): Partial<State> {
    return { failed: true };
  }

  componentDidCatch(): void {
    this.setState({ text: this.props.onCrash() });
  }

  render(): ReactNode {
    if (this.state.failed) return <ViewError error={this.state.text} />;
    return this.props.children;
  }
}
