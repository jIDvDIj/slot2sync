export type Route =
  | { name: "overview" }
  | { name: "emulator"; emulator: string }
  | { name: "activity" }
  | { name: "settings" };

export function routeKey(route: Route): string {
  return route.name === "emulator" ? `emulator:${route.emulator}` : route.name;
}
