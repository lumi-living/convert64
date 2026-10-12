# The No Glory title screen, built from its project with convert64 alone: no GUI (#167).
#
#   make                                  build/title.kla, .prg, .png and KickAssembler source
#   make check                            ... and validate them
#   make CONVERT64=../../target/debug/convert64
#
# Edit art/title.convert64 (layers, recipes, masks, palette) or art/title.png and run
# make again; docs/reference/project-file.md and docs/reference/cli.md describe both.

CONVERT64 ?= convert64

SOURCES := art/title.convert64 art/title.png

# A failed conversion (exit 1: too many clashes) must not leave a target behind that
# looks up to date.
.DELETE_ON_ERROR:

.PHONY: all check clean

all: build/title.kla build/title.prg build/title.png build/title.asm

build:
	mkdir -p build

# Koala Painter file: load it in any Koala viewer, or include it in a demo.
build/title.kla: $(SOURCES) | build
	$(CONVERT64) convert $< -o $@

# Self-displaying program: x64sc build/title.prg, or LOAD"TITLE",8 and RUN.
build/title.prg: $(SOURCES) | build
	$(CONVERT64) convert $< -o $@

# A preview at twice the size, every pixel a palette colour.
build/title.png: $(SOURCES) | build
	$(CONVERT64) convert $< -o $@ --zoom 2

# KickAssembler source including the raw set (title.bitmap.bin, title.screen.bin,
# title.colour.bin, title.bg.bin), all written next to it. Use --asm acme, 64tass,
# ca65 or c for the other assemblers.
build/title.asm: $(SOURCES) | build
	$(CONVERT64) export $< --asm kickass --out build

check: all
	$(CONVERT64) validate build/title.kla
	$(CONVERT64) validate build/title.prg

clean:
	rm -rf build
