type InitialTaskScrollState = {
  pendingInitialScroll: boolean;
  activeTab: "conversation" | "diff" | "terminal";
  persistedOutputLoading: boolean;
  renderedEventCount: number;
  totalEvents: number;
};

export function shouldFinalizeInitialTaskScroll(state: InitialTaskScrollState): boolean {
  if (!state.pendingInitialScroll) {
    return false;
  }
  if (state.activeTab !== "conversation") {
    return false;
  }
  if (state.persistedOutputLoading) {
    return false;
  }
  return state.renderedEventCount >= state.totalEvents;
}
