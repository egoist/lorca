package main

import (
	"math"
	"sync"
	"time"
)

// A working character's motion, after Grok Bot's: while its bot has a turn running it acts out
// short clips, one after another; at rest it stands still. The character is drawn as a head in
// 3D: the body turns (yaw), nods (pitch), and leans (roll), so the eyes slide across the face as
// it looks about and go behind it in a spin or a somersault, and the face changes between a few
// expressions. The clips are Lorca's own; characters_test.go writes them out for the phone app.

// pose is how a character stands at a moment.
type pose struct {
	// yaw turns the head right, pitch tips it up, and roll leans it clockwise, in degrees.
	yaw, pitch, roll float64
	// x and y move it in the box, scale grows it.
	x, y, scale float64
	face        expression
	// lids is how shut the eyes are, 0 open, 1 shut.
	lids float64
}

// expression is the eyes' shape: capsules `length` long and `width` wide in the box, tops leaning
// `tilt` degrees to the left, drawn from where the shape puts them (`center` 0) to the middle of
// the face (`center` 1).
type expression struct {
	length, width, tilt, center float64
}

var expressions = map[string]expression{
	// rest is the still character's: glancing up and to the right.
	"rest":      {0.43, 0.17, 25, 0},
	"neutral":   {0.46, 0.19, 0, 1},
	"surprised": {0.36, 0.27, 0, 1},
	"calm":      {0.32, 0.18, 90, 1},
}

// restPose is a character standing still.
var restPose = pose{scale: 1, face: expressions["rest"]}

// key is a value of a clip's track at time `t`, in seconds into the clip.
type key struct {
	t float64
	v [3]float64
}

// faceKey turns the face to an expression at time `t`.
type faceKey struct {
	t    float64
	name string
}

// clip is a few seconds of motion. Every clip starts and ends at rest, so they follow one another
// without a jump.
type clip struct {
	name     string
	duration float64
	// turn is yaw, pitch, and roll; move is x, y, and scale.
	turn, move []key
	face       []faceKey
	blinks     []float64
}

const (
	// faceMorph is how long the eyes take to change shape, and blinkLength one blink.
	faceMorph   = 0.28
	blinkLength = 0.26
)

var clips = []clip{
	{
		name: "look", duration: 5,
		turn:   []key{{0, [3]float64{}}, {0.7, [3]float64{-28, 6, -6}}, {1.7, [3]float64{-28, 6, -6}}, {2.3, [3]float64{24, -8, 8}}, {3.3, [3]float64{24, -8, 8}}, {3.9, [3]float64{0, 10, 0}}, {4.6, [3]float64{}}},
		move:   []key{{0, [3]float64{0, 0, 1}}},
		face:   []faceKey{{0, "rest"}, {0.3, "neutral"}, {2.3, "surprised"}, {3.3, "neutral"}, {4.6, "rest"}},
		blinks: []float64{1.2, 3.6},
	},
	{
		name: "sway", duration: 5,
		turn:   []key{{0, [3]float64{}}, {0.5, [3]float64{-6, 10, -30}}, {1.6, [3]float64{6, 10, 30}}, {2.7, [3]float64{-6, 10, -30}}, {3.8, [3]float64{6, 10, 30}}, {4.6, [3]float64{}}},
		move:   []key{{0, [3]float64{0, 0, 1}}, {0.5, [3]float64{-0.06, 0, 1}}, {1.6, [3]float64{0.04, 0, 1}}, {2.7, [3]float64{-0.06, 0, 1}}, {3.8, [3]float64{0.04, 0, 1}}, {4.6, [3]float64{0, 0, 1}}},
		face:   []faceKey{{0, "rest"}, {0.2, "surprised"}, {4.4, "rest"}},
		blinks: []float64{2.2},
	},
	{
		name: "flip", duration: 5,
		turn:   []key{{0, [3]float64{}}, {0.8, [3]float64{-22, -10, -12}}, {1.4, [3]float64{-22, -10, -12}}, {2, [3]float64{18, -12, 9}}, {2.5, [3]float64{0, -20, 0}}, {3.4, [3]float64{0, -340, 0}}, {3.8, [3]float64{0, -360, 0}}},
		move:   []key{{0, [3]float64{0, 0, 1}}, {2.5, [3]float64{0, 0.02, 1}}, {2.9, [3]float64{0, -0.2, 1}}, {3.5, [3]float64{0, -0.12, 1}}, {4, [3]float64{0, 0, 1}}},
		face:   []faceKey{{0, "rest"}, {0.3, "neutral"}, {1.9, "surprised"}, {4, "neutral"}, {4.6, "rest"}},
		blinks: []float64{1.1},
	},
	{
		name: "spin", duration: 4.8,
		turn:   []key{{0, [3]float64{}}, {1.3, [3]float64{-16, 2, -30}}, {2.6, [3]float64{16, -4, 22}}, {3.2, [3]float64{30, 0, 22}}, {3.9, [3]float64{-270, 0, 6}}, {4.6, [3]float64{-360, 0, 0}}},
		move:   []key{{0, [3]float64{0, 0, 1}}, {1.3, [3]float64{0, 0, 0.72}}, {2.2, [3]float64{0, 0, 0.86}}, {3, [3]float64{0, 0, 1.03}}, {4.5, [3]float64{0, 0, 1}}},
		face:   []faceKey{{0, "rest"}, {0.4, "neutral"}, {2.5, "surprised"}, {3.9, "neutral"}, {4.5, "rest"}},
		blinks: []float64{0.9},
	},
	{
		name: "glance", duration: 5,
		turn: []key{{0, [3]float64{}}, {0.5, [3]float64{-24, 4, -4}}, {1.5, [3]float64{-22, -6, -6}}, {2.2, [3]float64{24, 8, 10}}, {2.6, [3]float64{-26, 4, -8}}, {3.5, [3]float64{0, 6, 0}}, {4.2, [3]float64{24, 6, 8}}, {4.8, [3]float64{}}},
		move: []key{{0, [3]float64{0, 0, 1}}, {1.5, [3]float64{0, 0, 1}}, {2.2, [3]float64{0.1, 0.06, 1}}, {2.6, [3]float64{0, 0, 1}}},
		face: []faceKey{{0, "rest"}, {0.3, "calm"}, {4.6, "rest"}},
	},
}

// ease is smoothstep.
func ease(u float64) float64 { return u * u * (3 - 2*u) }

// track is a track's value at `t`, eased between the keys around it.
func track(keys []key, t float64) [3]float64 {
	if t <= keys[0].t {
		return keys[0].v
	}
	for i := 1; i < len(keys); i++ {
		if t < keys[i].t {
			a, b := keys[i-1], keys[i]
			u := ease((t - a.t) / (b.t - a.t))
			return [3]float64{a.v[0] + (b.v[0]-a.v[0])*u, a.v[1] + (b.v[1]-a.v[1])*u, a.v[2] + (b.v[2]-a.v[2])*u}
		}
	}
	return keys[len(keys)-1].v
}

// face is the eyes' expression at `t`: the last one turned to, morphing in from the one before.
func face(keys []faceKey, t float64) expression {
	from, to, since := keys[0].name, keys[0].name, 0.0
	for i, k := range keys {
		if k.t > t {
			break
		}
		if i > 0 {
			from = keys[i-1].name
		}
		to, since = k.name, t-k.t
	}
	a, b := expressions[from], expressions[to]
	u := ease(min(since/faceMorph, 1))
	return expression{a.length + (b.length-a.length)*u, a.width + (b.width-a.width)*u, a.tilt + (b.tilt-a.tilt)*u, a.center + (b.center-a.center)*u}
}

// clipPose is the pose `t` seconds into a clip.
func clipPose(c clip, t float64) pose {
	turn, move := track(c.turn, t), track(c.move, t)
	at := pose{yaw: turn[0], pitch: turn[1], roll: turn[2], x: move[0], y: move[1], scale: move[2], face: face(c.face, t)}
	for _, blink := range c.blinks {
		if since := t - blink; since >= 0 && since < blinkLength {
			at.lids = math.Sin(math.Pi * since / blinkLength)
		}
	}
	return at
}

// clipOrder is the order a bot plays the clips in, shuffled by its seed.
func clipOrder(seed uint32) []int {
	order := make([]int, len(clips))
	for i := range order {
		order[i] = i
	}
	state := seed | 1
	for i := len(order) - 1; i > 0; i-- {
		state ^= state << 13
		state ^= state >> 17
		state ^= state << 5
		j := int(state % uint32(i+1))
		order[i], order[j] = order[j], order[i]
	}
	return order
}

// workingPose is the pose `elapsed` seconds after a character started working: its clips in its
// order, round and round.
func workingPose(elapsed float64, seed uint32) pose {
	order := clipOrder(seed)
	var loop float64
	for _, i := range order {
		loop += clips[i].duration
	}
	t := math.Mod(elapsed, loop)
	for _, i := range order {
		if t < clips[i].duration {
			return clipPose(clips[i], t)
		}
		t -= clips[i].duration
	}
	return restPose
}

// motionSeed is the number a key (a bot's id) shuffles its character's clips by.
func motionSeed(key string) uint32 {
	hash := uint32(2166136261)
	for i := 0; i < len(key); i++ {
		hash ^= uint32(key[i])
		hash *= 16777619
	}
	return hash
}

// workStarts holds when each working character started and was last drawn working, by key, so
// its clips begin at the start and every avatar of one bot moves together.
var workStarts = struct {
	sync.Mutex
	at map[string][2]time.Time
}{at: map[string][2]time.Time{}}

// workStart is when the character under `key`, working, started: now, unless it was drawn working
// within the last second.
func workStart(key string, now time.Time) time.Time {
	workStarts.Lock()
	defer workStarts.Unlock()
	seen, ok := workStarts.at[key]
	if !ok || now.Sub(seen[1]) > time.Second {
		seen[0] = now
	}
	seen[1] = now
	workStarts.at[key] = seen
	return seen[0]
}
