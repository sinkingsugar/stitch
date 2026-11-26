/** Split only at the first slash: some providers' model IDs have slashes. */
export function modelName(model: string): string {
  return model.slice(model.indexOf("/") + 1);
}

/** Keep the node's ordering, including inside each provider. */
export function modelGroups(models: string[]): Map<string, string[]> {
  const groups = new Map<string, string[]>();
  for (const model of models) {
    const slash = model.indexOf("/");
    const provider = slash < 0 ? model : model.slice(0, slash);
    const list = groups.get(provider);
    if (list) list.push(model);
    else groups.set(provider, [model]);
  }
  return groups;
}
