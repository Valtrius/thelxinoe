import { toolsApi, type MpvSchema } from './tools-api';

const schemas = new Map<string, Promise<MpvSchema>>();

export function loadMpvSchema(
  key: string,
  refresh = false,
  load = toolsApi.schema,
): Promise<MpvSchema> {
  if (refresh) schemas.delete(key);
  let pending = schemas.get(key);
  if (!pending) {
    pending = load().catch((error: unknown) => {
      if (schemas.get(key) === pending) schemas.delete(key);
      throw error;
    });
    schemas.set(key, pending);
  }
  return pending;
}
