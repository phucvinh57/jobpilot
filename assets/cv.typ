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

#show: resume.with(
  author: b.name,
  // the package has no headline slot; it renders `pronouns` first in the contact line
  pronouns: b.at("headline", default: ""),
  location: b.at("location", default: ""),
  email: b.at("email", default: ""),
  github: b.at("github", default: ""),
  linkedin: b.at("linkedin", default: ""),
  phone: b.at("phone", default: ""),
  personal-site: b.at("website", default: ""),
  accent-color: style.at("accent-color", default: "#0092b8"),
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
      company: if e.at("url", default: "") != "" { link("https://" + e.url, e.company) } else { e.company },
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
      url: p.at("url", default: ""),
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
    block(certificates(name: c.name, issuer: c.at("issuer", default: ""), url: c.at("url", default: ""), date: c.at("date", default: "")))
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
