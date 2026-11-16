// Renders a CV from a YAML data file.
// Usage: typst compile --root <dir> --input data=<file>.yaml cv.typ out.pdf
#import "@preview/basic-resume:0.2.10": *

#let data = yaml("/" + sys.inputs.at("data", default: "cv.yaml"))
#let b = data.basics
#let style = data.at("style", default: (:))

// Text fields are Typst markup (*bold*, _italic_, #link("https://...")[text]).
#let md(s) = eval(str(s), mode: "markup")
#let visible(items) = items.filter(i => not i.at("hidden", default: false))
#let dates(i) = dates-helper(start-date: i.at("start", default: ""), end-date: i.at("end", default: ""))
#let bullets(xs) = if xs.len() > 0 { list(..xs.map(md)) }

#let default-titles = (
  summary: "Summary",
  skills: "Skills",
  experience: "Experience",
  projects: "Projects",
  education: "Education",
  certifications: "Certifications",
  languages: "Languages",
)
#let titles = default-titles + data.at("titles", default: (:))

// `rgb` refuses anything but a colour, so the hex below is safe to put in SVG markup.
#let accent = rgb(style.at("accent-color", default: "#0092b8")).to-hex()

// A profile link as `host/path`: any scheme, `www.` and trailing `/` dropped; a bare handle
// (`phucvinh57`, `@phucvinh57`) gets the site's prefix (`github.com/`).
#let bare-url(s, handle-prefix: "") = {
  let u = str(s).trim()
  for p in ("https://", "http://", "www.") {
    if lower(u).starts-with(p) { u = u.slice(p.len()) }
  }
  u = u.trim("/", at: end)
  if handle-prefix != "" and u != "" and not u.contains("/") and not u.contains(".") {
    u = handle-prefix + u.trim("@", at: start)
  }
  u
}

// Brand marks (Simple Icons, CC0), drawn in the accent colour.
#let icon-paths = (
  github: "M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12",
  linkedin: "M20.447 20.452h-3.554v-5.569c0-1.328-.027-3.037-1.852-3.037-1.853 0-2.136 1.445-2.136 2.939v5.667H9.351V9h3.414v1.561h.046c.477-.9 1.637-1.85 3.37-1.85 3.601 0 4.267 2.37 4.267 4.9v4.837zM5.337 7.433c-1.144 0-2.063-.926-2.063-2.065 0-1.138.92-2.063 2.063-2.063 1.14 0 2.064.925 2.064 2.063 0 1.139-.925 2.065-2.064 2.065zm1.782 13.019H3.555V9h3.564v11.452zM22.225 0H1.771C.792 0 0 .774 0 1.729v20.542C0 23.227.792 24 1.771 24h20.451C23.2 24 24 23.227 24 22.271V1.729C24 .774 23.2 0 22.222 0h.003z",
)
#let icon(name) = box(baseline: 0.12em, image(
  bytes("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24'><path fill='" + accent + "' d='" + icon-paths.at(name) + "'/></svg>"),
  format: "svg",
  height: 0.9em,
  alt: name,
))

#let web-link(field, handle-prefix: "", icon-name: none) = {
  let u = bare-url(b.at(field, default: ""), handle-prefix: handle-prefix)
  if u != "" {
    // a word joiner keeps the icon on the line of the text; a long URL can still wrap
    link("https://" + u)[#if icon-name != none [#icon(icon-name)#h(0.25em)#sym.wj]#u]
  }
}

// The whole contact line, built here so links get a scheme exactly once and an icon. The package
// has no headline slot and renders `pronouns` first, so the line goes there.
#let contact-line = {
  let text-of(k) = str(b.at(k, default: "")).trim()
  let items = (
    text-of("headline"),
    if text-of("phone") != "" { link("tel:" + text-of("phone").replace(" ", ""), text-of("phone")) },
    text-of("location"),
    if text-of("email") != "" { link("mailto:" + text-of("email"), text-of("email")) },
    web-link("github", handle-prefix: "github.com/", icon-name: "github"),
    web-link("linkedin", handle-prefix: "linkedin.com/in/", icon-name: "linkedin"),
    web-link("website"),
  )
  items.filter(x => x != none and x != "").join("  |  ")
}

#show: resume.with(
  author: b.name,
  pronouns: if contact-line == none { "" } else { contact-line },
  accent-color: accent,
  font: style.at("font", default: "New Computer Modern"),
  font-size: eval(style.at("font-size", default: "10pt")),
  paper: style.at("paper", default: "a4"),
)

#let render-summary() = bullets(data.summary)

#let render-skills() = {
  for s in visible(data.skills) [
    - *#s.category*: #s.items.join(", ")
  ]
}

#let render-experience() = {
  for e in visible(data.experience) {
    block(work(
      title: e.position,
      company: if bare-url(e.at("url", default: "")) != "" { link("https://" + bare-url(e.url), e.company) } else { e.company },
      location: e.at("location", default: ""),
      dates: dates(e),
    ))
    bullets(e.at("bullets", default: ()))
    for g in e.at("groups", default: ()) {
      block(emph(md(g.title)))
      bullets(g.bullets)
    }
  }
}

#let render-projects() = {
  for p in visible(data.projects) {
    block(project(
      name: p.name,
      role: p.at("role", default: ""),
      url: bare-url(p.at("url", default: "")),
      dates: dates(p),
    ))
    let intro = p.at("intro", default: "")
    if intro != "" { block(md(intro)) }
    bullets(p.at("bullets", default: ()))
  }
}

#let render-education() = {
  for e in visible(data.education) {
    block(edu(
      institution: e.institution,
      degree: e.at("degree", default: ""),
      location: e.at("location", default: ""),
      gpa: e.at("gpa", default: ""),
      dates: dates(e),
      consistent: true,
    ))
    bullets(e.at("bullets", default: ()))
  }
}

#let render-certifications() = {
  for c in visible(data.certifications) {
    block(certificates(name: c.name, issuer: c.at("issuer", default: ""), url: bare-url(c.at("url", default: "")), date: c.at("date", default: "")))
  }
}

#let render-languages() = {
  for l in visible(data.languages) [
    - *#l.language*: #l.fluency
  ]
}

#let renderers = (
  summary: render-summary,
  skills: render-skills,
  experience: render-experience,
  projects: render-projects,
  education: render-education,
  certifications: render-certifications,
  languages: render-languages,
)

#for key in data.at("sections", default: renderers.keys()) {
  if key in data and data.at(key) != none and data.at(key).len() > 0 [
    == #titles.at(key)
    #renderers.at(key)()
  ]
}
