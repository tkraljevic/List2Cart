import type { ShoppingList } from "../../types";

export interface ListCreationApi {
  create(name: string, storeId: number | null, shared: boolean): Promise<ShoppingList>;
  refresh(): Promise<ShoppingList[]>;
}

/** A failed refresh must not turn a confirmed save into a failed create/retry. */
export async function createListWithRefresh(name: string, storeId: number | null, api: ListCreationApi, shared = false) {
  const created = await api.create(name.trim(), storeId, shared);
  try {
    return { created, lists: await api.refresh(), refreshError: null };
  } catch (error) {
    return { created, lists: null, refreshError: String(error) };
  }
}

export function listCreationError(error: unknown): string {
  const message = String(error);
  if (message.includes("Zajednicki popis moze stvoriti samo")) {
    return "Zajednički popis može stvoriti samo uređaj koji upravlja mrežom. Stvorite ga ondje ili stvorite običan popis.";
  }
  if (message.includes("Zajednicki popis moze obrisati samo")) {
    return "Zajednički popis može obrisati samo uređaj koji upravlja mrežom, jer bi nestao svima.";
  }
  if (message.includes("Dijeliti popise moze samo")) {
    return "Dijeliti popise može samo uređaj koji upravlja mrežom.";
  }
  if (message.includes("Privatni kanal dopusten")) {
    return "Ovaj uređaj nema pravo pisati osobne popise ove osobe. Povežite ga ponovno s uređajem koji upravlja mrežom.";
  }
  return `Popis nije spremljen: ${message}`;
}
