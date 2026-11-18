import { redirect } from "@sveltejs/kit";

export const load = () => {
  if (!import.meta.env.DEV) redirect(307, "/");
};
