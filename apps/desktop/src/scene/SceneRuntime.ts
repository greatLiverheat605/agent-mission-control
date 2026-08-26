export type Disposable = { dispose: () => void };

export class SceneRuntime {
  private readonly resources = new Set<Disposable>();
  private readonly listeners = new Set<() => void>();
  private disposed = false;

  own<T extends Disposable>(resource: T): T {
    if (this.disposed) resource.dispose();
    else this.resources.add(resource);
    return resource;
  }

  listen(remove: () => void): () => void {
    if (this.disposed) remove();
    else this.listeners.add(remove);
    return remove;
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    for (const remove of this.listeners) remove();
    for (const resource of this.resources) resource.dispose();
    this.listeners.clear();
    this.resources.clear();
  }
}
