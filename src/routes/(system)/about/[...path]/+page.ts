import { redirect } from "@sveltejs/kit";

export const load = ({ params, url }) => {
  const rest = params.path ? `/${params.path}` : "/about";
  redirect(308, `/community${rest}${url.search}`);
};
