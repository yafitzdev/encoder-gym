/** One cancellable setup session, spanning the commands before a run UUID exists. */
export class OptimizationPreparation {
  private sessions = new Map<string, { token: string; controller: AbortController }>();
  async command<T>(projectId: string, token: unknown, action: (signal?: AbortSignal) => Promise<T>): Promise<T> {
    if (token === undefined) return action();
    const session = this.session(projectId, token);
    session.controller.signal.throwIfAborted();
    return action(session.controller.signal);
  }
  stop(projectId: string, token: unknown): void { this.session(projectId, token).controller.abort(); }
  finish(projectId: string, token: unknown): void {
    if (this.sessions.get(projectId)?.token === token) this.sessions.delete(projectId);
  }
  private session(projectId: string, token: unknown) {
    if (typeof token !== "string" || !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(token)) throw new Error("Invalid preparation identity.");
    const current = this.sessions.get(projectId);
    if (current && current.token !== token) throw new Error("Another setup is active for this project.");
    if (current) return current;
    const session = { token, controller: new AbortController() };
    this.sessions.set(projectId, session);
    return session;
  }
}
