/**
 * Pure state mapping for the admin managers page (WEB-03 / #804).
 *
 * Authorization errors are deliberately distinct from service errors, and no
 * branch treats an empty array as proof of authorization. The API is the only
 * authorization boundary; these helpers only describe its result.
 */

export type ManagersViewState =
  | "loading"
  | "populated"
  | "empty"
  | "forbidden"
  | "unauthenticated"
  | "service-error";

export interface HttpErrorLike {
  status: number;
}

function hasStatus(error: unknown): error is HttpErrorLike {
  return (
    typeof error === "object" &&
    error !== null &&
    "status" in error &&
    typeof (error as HttpErrorLike).status === "number"
  );
}

export function stateFromLoadResult(rowCount: number): ManagersViewState {
  return rowCount > 0 ? "populated" : "empty";
}

export function stateFromLoadError(error: unknown): ManagersViewState {
  if (hasStatus(error) && error.status === 401) return "unauthenticated";
  if (hasStatus(error) && error.status === 403) return "forbidden";
  return "service-error";
}
