package main

import (
	"bytes"
	"fmt"
	"image"
	"image/color"
	"image/png"
	"math"
	"os"
	"path/filepath"
	"regexp"
	"slices"
	"strconv"
	"strings"
	"testing"

	"golang.org/x/image/vector"
)

// The phone app and the website draw the characters from files made here: a white mask per shape
// for the phone to tint, and tables of eyes (and, for the website, outlines). They are checked in;
// LORCA_WRITE_CHARACTERS=1 go test -run TestCharacterFiles . rewrites them after a change here.

// maskSide is a phone mask's side in pixels, an 80-point avatar at 3×.
const maskSide = 240

func TestShapesMatchTheCLI(t *testing.T) {
	source, err := os.ReadFile("../crates/cli/src/model.rs")
	if err != nil {
		t.Fatal(err)
	}
	list := regexp.MustCompile(`(?s)BOT_SHAPES: \[&str; \d+\] = \[(.*?)\];`).FindSubmatch(source)
	if list == nil {
		t.Fatal("BOT_SHAPES not found in model.rs")
	}
	var names []string
	for _, m := range regexp.MustCompile(`"([a-z]+)"`).FindAllSubmatch(list[1], -1) {
		names = append(names, string(m[1]))
	}
	if !slices.Equal(names, botShapes) {
		t.Fatalf("BOT_SHAPES is %v, the app draws %v", names, botShapes)
	}
}

func TestShapeForKeepsAGivenShapeAndSpreadsTheRest(t *testing.T) {
	if got := shapeFor("heart", "bot-1"); got != "heart" {
		t.Fatalf("a given shape is kept, got %s", got)
	}
	// The phone app's shapeFor hashes the same way (mobile/src/ui/Avatar.tsx).
	if got := shapeFor("", "bot-0123456789abcdef"); got != "pebble" {
		t.Fatalf("bot-0123456789abcdef picks %s", got)
	}
	seen := map[string]bool{}
	for i := range 200 {
		seen[shapeFor("unknown", fmt.Sprintf("bot-%d", i))] = true
	}
	if len(seen) != len(botShapes) {
		t.Fatalf("200 ids picked %d of %d shapes", len(seen), len(botShapes))
	}
}

func TestEveryCharacterFillsItsBoxAndKeepsItsEyesInside(t *testing.T) {
	for _, shape := range botShapes {
		made := characters[shape]
		minX, minY, maxX, maxY := math.Inf(1), math.Inf(1), math.Inf(-1), math.Inf(-1)
		for _, p := range made.outline {
			minX, maxX = min(minX, p[0]), max(maxX, p[0])
			minY, maxY = min(minY, p[1]), max(maxY, p[1])
		}
		if math.Abs(max(maxX-minX, maxY-minY)-2*bodyReach) > 1e-9 {
			t.Errorf("%s spans %.3f×%.3f", shape, maxX-minX, maxY-minY)
		}
		// Each eye, with a margin of half its width again, is on the body.
		for _, eye := range made.eyes {
			for step := range 9 {
				u := float64(step) / 8
				x, y := eye[0][0]+u*(eye[1][0]-eye[0][0]), eye[0][1]+u*(eye[1][1]-eye[0][1])
				for k := range 8 {
					s, c := math.Sincos(2 * math.Pi * float64(k) / 8)
					if !inside(made.outline, x+c*eyeWidth, y+s*eyeWidth) {
						t.Errorf("%s: an eye reaches past the body at (%.2f, %.2f)", shape, x, y)
						break
					}
				}
			}
		}
	}
}

func inside(polygon [][2]float64, x, y float64) bool {
	in := false
	for i, a := range polygon {
		b := polygon[(i+len(polygon)-1)%len(polygon)]
		if (a[1] > y) != (b[1] > y) && x < (b[0]-a[0])*(y-a[1])/(b[1]-a[1])+a[0] {
			in = !in
		}
	}
	return in
}

func TestCharacterFiles(t *testing.T) {
	files := map[string][]byte{
		"../mobile/src/ui/characters.ts":           phoneTable(),
		"../web/src/components/site/characters.ts": webTable(),
	}
	for _, shape := range botShapes {
		files["../mobile/assets/shapes/"+shape+".png"] = mask(characters[shape])
	}
	write := os.Getenv("LORCA_WRITE_CHARACTERS") == "1"
	for path, want := range files {
		if write {
			if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
				t.Fatal(err)
			}
			if err := os.WriteFile(path, want, 0o644); err != nil {
				t.Fatal(err)
			}
			continue
		}
		got, err := os.ReadFile(path)
		if err != nil || !closeEnough(path, got, want) {
			t.Errorf("%s is not what characters.go makes; run LORCA_WRITE_CHARACTERS=1 go test -run TestCharacterFiles .", path)
		}
	}
	if dir := os.Getenv("LORCA_RENDER"); dir != "" {
		if err := os.WriteFile(filepath.Join(dir, "characters.png"), preview(), 0o644); err != nil {
			t.Fatal(err)
		}
	}
}

// closeEnough is whether a checked-in file is what characters.go makes, but for the last digit:
// arm64 fuses multiply-adds that amd64 rounds apart, so a mask's edge pixel or a path's last
// decimal can differ by a step between the computer that wrote the file and the one checking it.
func closeEnough(path string, got, want []byte) bool {
	if strings.HasSuffix(path, ".png") {
		a, errA := png.Decode(bytes.NewReader(got))
		b, errB := png.Decode(bytes.NewReader(want))
		if errA != nil || errB != nil || a.Bounds() != b.Bounds() {
			return false
		}
		for y := a.Bounds().Min.Y; y < a.Bounds().Max.Y; y++ {
			for x := a.Bounds().Min.X; x < a.Bounds().Max.X; x++ {
				_, _, _, alphaA := a.At(x, y).RGBA()
				_, _, _, alphaB := b.At(x, y).RGBA()
				if math.Abs(float64(alphaA)-float64(alphaB)) > 4*257 {
					return false
				}
			}
		}
		return true
	}
	numbers := regexp.MustCompile(`-?\d+(\.\d+)?`)
	if numbers.ReplaceAllString(string(got), "0") != numbers.ReplaceAllString(string(want), "0") {
		return false
	}
	a, b := numbers.FindAllString(string(got), -1), numbers.FindAllString(string(want), -1)
	for i := range a {
		x, _ := strconv.ParseFloat(a[i], 64)
		y, _ := strconv.ParseFloat(b[i], 64)
		if math.Abs(x-y) > 0.11 {
			return false
		}
	}
	return true
}

const generatedHeader = "// Made by desktop/characters_test.go from desktop/characters.go; rewrite it with\n" +
	"// LORCA_WRITE_CHARACTERS=1 go test -run TestCharacterFiles . in desktop/, never by hand.\n\n"

// eyeTable is each shape's eyes as their middles, in the box from -1 to 1.
func eyeTable(b *strings.Builder, typeName string) {
	fmt.Fprintf(b, "/// Each eye's middle in a box from -1 to 1, y down. An eye is EYE_WIDTH wide and EYE_LENGTH\n")
	fmt.Fprintf(b, "/// long in that box, with round ends, its top leaning EYE_TILT degrees to the left.\n")
	fmt.Fprintf(b, "export const EYE_WIDTH = %s;\n", number(eyeWidth))
	fmt.Fprintf(b, "export const EYE_LENGTH = %s;\n", number(2*eyeReach+eyeWidth))
	fmt.Fprintf(b, "export const EYE_TILT = %s;\n\n", number(eyeTilt*180/math.Pi))
	fmt.Fprintf(b, "export const SHAPE_EYES: Record<%s, [number, number][]> = {\n", typeName)
	for _, shape := range botShapes {
		var middles []string
		for _, eye := range characters[shape].eyes {
			middles = append(middles, fmt.Sprintf("[%s, %s]", number((eye[0][0]+eye[1][0])/2), number((eye[0][1]+eye[1][1])/2)))
		}
		fmt.Fprintf(b, "  %s: [%s],\n", shape, strings.Join(middles, ", "))
	}
	b.WriteString("};\n")
}

func shapeList(b *strings.Builder) {
	var quoted []string
	for _, shape := range botShapes {
		quoted = append(quoted, `"`+shape+`"`)
	}
	fmt.Fprintf(b, "export const BOT_SHAPES = [%s] as const;\n", strings.Join(quoted, ", "))
	b.WriteString("export type BotShape = (typeof BOT_SHAPES)[number];\n\n")
}

func phoneTable() []byte {
	var b strings.Builder
	b.WriteString(generatedHeader)
	b.WriteString("// Bot characters' bodies as white masks to tint, and where their eyes sit.\n\n")
	shapeList(&b)
	b.WriteString("export const SHAPE_MASKS: Record<BotShape, number> = {\n")
	for _, shape := range botShapes {
		fmt.Fprintf(&b, "  %s: require(\"../../assets/shapes/%s.png\"),\n", shape, shape)
	}
	b.WriteString("};\n\n")
	eyeTable(&b, "BotShape")
	return []byte(b.String())
}

func webTable() []byte {
	var b strings.Builder
	b.WriteString(generatedHeader)
	b.WriteString("// Bot characters' bodies as SVG paths in a 100 × 100 box, and where their eyes sit.\n\n")
	shapeList(&b)
	b.WriteString("export const SHAPE_PATHS: Record<BotShape, string> = {\n")
	for _, shape := range botShapes {
		var d strings.Builder
		for i, p := range characters[shape].outline {
			// Every other point is plenty at the sizes the site shows.
			if i%2 == 1 {
				continue
			}
			op := "L"
			if i == 0 {
				op = "M"
			}
			fmt.Fprintf(&d, "%s%s %s", op, strings.TrimSuffix(fmt.Sprintf("%.1f", 50+p[0]*50), ".0"), strings.TrimSuffix(fmt.Sprintf("%.1f", 50+p[1]*50), ".0"))
		}
		d.WriteString("Z")
		fmt.Fprintf(&b, "  %s: %q,\n", shape, d.String())
	}
	b.WriteString("};\n\n")
	eyeTable(&b, "BotShape")
	// The site's own style: single quotes, no semicolons.
	text := strings.ReplaceAll(b.String(), `"`, "'")
	return []byte(strings.ReplaceAll(text, ";\n", "\n"))
}

func number(v float64) string {
	s := strings.TrimRight(strings.TrimRight(fmt.Sprintf("%.3f", v), "0"), ".")
	if s == "-0" {
		return "0"
	}
	return s
}

// mask is the body in white on clear, maskSide across.
func mask(made *character) []byte {
	alpha := image.NewAlpha(image.Rect(0, 0, maskSide, maskSide))
	fillOutline(alpha, made.outline, maskSide, 0)
	white := image.NewNRGBA(alpha.Bounds())
	for i, a := range alpha.Pix {
		copy(white.Pix[i*4:], []byte{255, 255, 255, a})
	}
	var out bytes.Buffer
	encoder := png.Encoder{CompressionLevel: png.BestCompression}
	if err := encoder.Encode(&out, white); err != nil {
		panic(err)
	}
	return out.Bytes()
}

// fillOutline rasterizes points from the box into a square `side` across at x offset `left`.
func fillOutline(dst *image.Alpha, points [][2]float64, side int, left int) {
	z := vector.NewRasterizer(dst.Bounds().Dx(), dst.Bounds().Dy())
	half := float32(side) / 2
	for i, p := range points {
		x, y := float32(left)+half+float32(p[0])*half, half+float32(p[1])*half
		if i == 0 {
			z.MoveTo(x, y)
		} else {
			z.LineTo(x, y)
		}
	}
	z.ClosePath()
	z.Draw(dst, dst.Bounds(), image.Opaque, image.Point{})
}

// preview is every character in a row, in the accents, for looking at.
func preview() []byte {
	const side = 128
	colors := []color.NRGBA{{88, 86, 214, 255}, {0, 122, 255, 255}, {48, 176, 199, 255}, {52, 199, 89, 255}, {255, 149, 0, 255}, {255, 45, 85, 255}, {175, 82, 222, 255}, {255, 59, 48, 255}}
	canvas := image.NewNRGBA(image.Rect(0, 0, side*len(botShapes), side))
	for i := range canvas.Pix {
		canvas.Pix[i] = 245
	}
	for i, shape := range botShapes {
		made := characters[shape]
		body := image.NewAlpha(canvas.Bounds())
		fillOutline(body, made.outline, side, i*side)
		eyes := image.NewAlpha(canvas.Bounds())
		for _, eye := range made.eyes {
			fillOutline(eyes, capsule(eye), side, i*side)
		}
		paint(canvas, body, colors[i%len(colors)])
		paint(canvas, eyes, color.NRGBA{255, 255, 255, 255})
	}
	var out bytes.Buffer
	_ = png.Encode(&out, canvas)
	return out.Bytes()
}

func capsule(eye [2][2]float64) [][2]float64 {
	dx, dy := eye[1][0]-eye[0][0], eye[1][1]-eye[0][1]
	base := math.Atan2(dy, dx)
	var points [][2]float64
	for end, at := range eye {
		for k := range 17 {
			a := base - math.Pi/2 + math.Pi*float64(k)/16
			if end == 0 {
				a += math.Pi
			}
			s, c := math.Sincos(a)
			points = append(points, [2]float64{at[0] + c*eyeWidth/2, at[1] + s*eyeWidth/2})
		}
	}
	return points
}

func paint(canvas *image.NRGBA, coverage *image.Alpha, ink color.NRGBA) {
	for i, a := range coverage.Pix {
		if a == 0 {
			continue
		}
		for k, v := range []uint8{ink.R, ink.G, ink.B} {
			old := canvas.Pix[i*4+k]
			canvas.Pix[i*4+k] = uint8((int(v)*int(a) + int(old)*(255-int(a))) / 255)
		}
	}
}
