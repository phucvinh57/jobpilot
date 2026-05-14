import { basename, extname, resolve } from "node:path";
import { readFileSync } from "node:fs";
import "dotenv/config";

type ResumeSummary = {
  id: string;
  name: string;
  slug: string;
};

function usage(): never {
  console.error("Usage: bun run cv:upload -- <path-to-cv.json>");
  process.exit(1);
}

function slugify(input: string): string {
  return (
    input
      .toLowerCase()
      .trim()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 80) || "resume"
  );
}

async function apiRequest<T>(
  baseUrl: string,
  apiKey: string,
  path: string,
  init: RequestInit = {},
): Promise<T> {
  const response = await fetch(`${baseUrl}/api/openapi${path}`, {
    ...init,
    headers: {
      "Content-Type": "application/json",
      "x-api-key": apiKey,
      ...(init.headers ?? {}),
    },
  });

  if (!response.ok) {
    const body = await response.text();
    throw new Error(`Request failed ${init.method ?? "GET"} ${path}: ${response.status} ${body}`);
  }

  return (await response.json()) as T;
}

async function main() {
  const inputPath = process.argv[2];
  if (!inputPath) usage();

  const apiKey = process.env.RXRESUME_API_KEY;
  if (!apiKey) {
    throw new Error("Missing RXRESUME_API_KEY in environment.");
  }

  const baseUrl = (process.env.RXRESUME_BASE_URL ?? process.env.APP_URL ?? "http://localhost:3000").replace(/\/$/, "");
  const absolutePath = resolve(inputPath);

  const raw = readFileSync(absolutePath, "utf-8");
  const resumeData = JSON.parse(raw);

  const fileName = basename(absolutePath, extname(absolutePath));
  const resumeName = fileName;
  const resumeSlug = slugify(fileName);

  const resumes = await apiRequest<ResumeSummary[]>(baseUrl, apiKey, "/resumes");
  let resumeId = resumes.find((r) => r.slug === resumeSlug)?.id;

  if (!resumeId) {
    resumeId = await apiRequest<string>(baseUrl, apiKey, "/resumes", {
      method: "POST",
      body: JSON.stringify({
        name: resumeName,
        slug: resumeSlug,
        tags: [],
        withSampleData: false,
      }),
    });
    console.log(`Created resume: ${resumeId}`);
  } else {
    console.log(`Found existing resume by slug '${resumeSlug}': ${resumeId}`);
  }

  await apiRequest(baseUrl, apiKey, `/resumes/${resumeId}`, {
    method: "PUT",
    body: JSON.stringify({
      name: resumeName,
      slug: resumeSlug,
      data: resumeData,
    }),
  });

  console.log(`Uploaded CV '${absolutePath}' to ${baseUrl} as '${resumeName}' (id: ${resumeId}).`);
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
});
