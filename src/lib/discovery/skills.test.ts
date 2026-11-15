import { describe, expect, it } from "vitest";

import { listOrNone, sameSkill, splitSkills } from "./skills";

describe("skills of a posting", () => {
  it("are split by the profile", () => {
    const wanted = { required: ["Rust", "PostgreSQL", "Kafka"], preferred: ["kubernetes", "rust"] };
    const profile = ["Rust", "PostgreSQL", "Kubernetes (EKS)"];
    expect(splitSkills(wanted, profile)).toEqual({ matched: ["Rust", "PostgreSQL", "kubernetes"], gaps: ["Kafka"] });
  });

  it("count once and blanks are ignored", () => {
    const split = splitSkills({ required: ["Go", " go ", "", "  "], preferred: ["GO"] }, []);
    expect(split.gaps).toEqual(["Go"]);
    expect(split.matched).toEqual([]);
  });

  it("similar names are not the same skill", () => {
    const wanted = { required: ["Java", "C", "SQL", "Cloud", "CI/CD", "REST"], preferred: [] };
    const profile = ["JavaScript", "C++", "PostgreSQL", "Google Cloud", "CI", "REST APIs", "Java Spring"];
    const split = splitSkills(wanted, profile);
    expect(split.matched).toEqual([]);
    expect(split.gaps).toEqual(["Java", "C", "SQL", "Cloud", "CI/CD", "REST"]);
  });

  it("a longer name does not match a part of it (both ways round)", () => {
    expect(sameSkill("Java", "Java Spring")).toBe(false);
    expect(sameSkill("Java Spring", "Java")).toBe(false);
    expect(sameSkill("Cloud", "Google Cloud")).toBe(false);
    expect(sameSkill("CI/CD", "CI")).toBe(false);
    expect(sameSkill("CI", "CI/CD")).toBe(false);
    expect(sameSkill("Terraform", "Terraform / OpenTofu"), "'/' and '&' do not split a name").toBe(false);
    expect(sameSkill("R&D", "R")).toBe(false);
  });

  it("ignore case, spacing and a trailing parenthetical", () => {
    expect(sameSkill("rust", "Rust")).toBe(true);
    expect(sameSkill("  Node.js ", "node.js")).toBe(true);
    expect(sameSkill("Apache   Kafka", "apache kafka")).toBe(true);
    expect(sameSkill("C++", "c++")).toBe(true);
    expect(sameSkill("CI/CD", "ci/cd")).toBe(true);
    expect(sameSkill("Kubernetes (EKS)", "Kubernetes")).toBe(true);
    expect(sameSkill("Kubernetes", "kubernetes  (EKS)")).toBe(true);
    expect(sameSkill("Postgres (RDS (Aurora))", "Postgres"), "a nested group goes with the rest").toBe(true);
    expect(sameSkill("C (embedded) and Rust", "C"), "only a parenthetical at the very end goes").toBe(false);
  });

  it("nothing matches a blank or a bare parenthetical", () => {
    expect(sameSkill("", "")).toBe(false);
    expect(sameSkill("  ", " ")).toBe(false);
    expect(sameSkill("(EKS)", "(GKE)"), "a name that is only a parenthetical is kept whole").toBe(false);
    expect(sameSkill("(EKS)", "(eks)")).toBe(true);
  });

  it("are listed, or none", () => {
    expect(listOrNone(["Rust", "Go"])).toBe("Rust, Go");
    expect(listOrNone([])).toBe("none");
  });
});
