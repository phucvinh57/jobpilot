TYPST    ?= typst
TEMPLATE := template/cv.typ
CVS      := $(basename $(notdir $(wildcard cvs/*.yaml)))

.PHONY: all profile cv watch clean list check hooks

# Build the master profile and every tailored CV
all: profile $(addprefix build/,$(addsuffix .pdf,$(CVS)))

# Full CV from profile.yaml
profile: build/profile.pdf
build/profile.pdf: profile.yaml $(TEMPLATE) | build
	$(TYPST) compile --root . --input data=profile.yaml $(TEMPLATE) $@

# make cv NAME=<name>  ->  cvs/<name>.yaml -> build/<name>.pdf
cv: build/$(NAME).pdf

build/%.pdf: cvs/%.yaml $(TEMPLATE) | build
	$(TYPST) compile --root . --input data=$< $(TEMPLATE) $@

# make watch NAME=<name>   (omit NAME to watch profile.yaml)
watch: | build
	$(TYPST) watch --root . --input data=$(if $(NAME),cvs/$(NAME).yaml,profile.yaml) $(TEMPLATE) build/$(or $(NAME),profile).pdf

list:
	@printf '%s\n' $(CVS)

build:
	mkdir -p build

clean:
	rm -rf build

# Run the pre-PR checks (make check ARGS=--all for everything)
check:
	scripts/check.sh $(ARGS)

# Enable the pre-push hook (once per clone)
hooks:
	git config core.hooksPath .githooks
