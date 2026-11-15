// Runs before every test file: toasts are global state, so each test starts without any.

import { afterEach } from "vitest";

import { useToasts } from "@/stores/toast";

afterEach(() => useToasts.getState().clear());
