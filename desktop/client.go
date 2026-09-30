package main

import (
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"fmt"
	"sync"
	"time"

	"github.com/coder/websocket"
)

// Errors the pages word themselves; each is the English key of its translation.
var (
	errNotRunning       = errors.New("The Lorca CLI is not running")
	errConnectionClosed = errors.New("The CLI connection closed")
	errConnectionDrop   = errors.New("The CLI connection dropped")
)

type reply struct {
	result jsontext.Value
	err    error
}

// cliClient is the app's only network client: a websocket to the local CLI on 127.0.0.1. It
// reconnects on its own, asking the launcher to make the endpoint ready first.
type cliClient struct {
	mu             sync.Mutex
	state          string // "disconnected", "connecting", "connected"
	conn           *websocket.Conn
	generation     int
	wants          bool
	pending        map[int64]chan reply
	nextID         int64
	reconnectDelay time.Duration
	reconnectTimer *time.Timer

	onState           func(state string)
	onEvent           func(name string, frame []byte)
	onReconnectNeeded func()
}

func newCLIClient() *cliClient {
	return &cliClient{state: "disconnected", pending: map[int64]chan reply{}, nextID: 1, reconnectDelay: 400 * time.Millisecond}
}

func (c *cliClient) currentState() string {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.state
}

func (c *cliClient) connect() {
	c.mu.Lock()
	c.wants = true
	if c.reconnectTimer != nil {
		c.reconnectTimer.Stop()
		c.reconnectTimer = nil
	}
	if c.conn != nil || c.state == "connecting" {
		c.mu.Unlock()
		return
	}
	c.mu.Unlock()
	c.open()
}

func (c *cliClient) disconnect() {
	c.mu.Lock()
	c.wants = false
	c.mu.Unlock()
	c.close(errConnectionClosed)
}

// reconnect drops the socket and dials again, for a port change or a manual retry.
func (c *cliClient) reconnect() {
	c.mu.Lock()
	c.wants = true
	c.reconnectDelay = 400 * time.Millisecond
	c.mu.Unlock()
	c.close(errConnectionClosed)
	c.requestReconnect()
}

func (c *cliClient) open() {
	c.mu.Lock()
	c.generation++
	generation := c.generation
	c.state = "connecting"
	url := fmt.Sprintf("ws://127.0.0.1:%d/ws", prefs.cliPort())
	c.mu.Unlock()

	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		conn, _, err := websocket.Dial(ctx, url, nil)
		cancel()
		if err != nil {
			c.dropped(generation)
			return
		}
		// A snapshot carries every chat's newest messages and outgrows any small limit.
		conn.SetReadLimit(256 * 1024 * 1024)
		c.mu.Lock()
		if generation != c.generation {
			c.mu.Unlock()
			conn.CloseNow()
			return
		}
		c.conn = conn
		c.reconnectDelay = 400 * time.Millisecond
		c.state = "connected"
		onState := c.onState
		c.mu.Unlock()
		startupTrace("websocket connected")
		if onState != nil {
			onState("connected")
		}
		c.read(generation, conn)
	}()
}

func (c *cliClient) read(generation int, conn *websocket.Conn) {
	for {
		_, data, err := conn.Read(context.Background())
		if err != nil {
			c.dropped(generation)
			return
		}
		c.handle(data)
	}
}

type frameError struct {
	Message string `json:"message"`
}

func (c *cliClient) handle(frame []byte) {
	var head struct {
		Event  string         `json:"event"`
		ID     *int64         `json:"id"`
		Error  *frameError    `json:"error"`
		Result jsontext.Value `json:"result"`
	}
	if json.Unmarshal(frame, &head) != nil {
		return
	}
	if head.Event != "" {
		if c.onEvent != nil {
			c.onEvent(head.Event, frame)
		}
		return
	}
	if head.ID == nil {
		return
	}
	c.mu.Lock()
	waiting, ok := c.pending[*head.ID]
	delete(c.pending, *head.ID)
	c.mu.Unlock()
	if !ok {
		return
	}
	if head.Error != nil {
		message := head.Error.Message
		if message == "" {
			message = "Request failed"
		}
		waiting <- reply{err: errors.New(message)}
		return
	}
	result := head.Result
	if len(result) == 0 {
		result = jsontext.Value("null")
	}
	waiting <- reply{result: result}
}

func (c *cliClient) dropped(generation int) {
	c.mu.Lock()
	if generation != c.generation {
		c.mu.Unlock()
		return
	}
	c.generation++
	c.conn = nil
	changed := c.state != "disconnected"
	c.state = "disconnected"
	waiting := c.takePendingLocked()
	onState := c.onState
	c.mu.Unlock()
	for _, ch := range waiting {
		ch <- reply{err: errConnectionDrop}
	}
	if changed && onState != nil {
		onState("disconnected")
	}
	c.scheduleReconnect()
}

func (c *cliClient) close(reason error) {
	c.mu.Lock()
	c.generation++
	if c.reconnectTimer != nil {
		c.reconnectTimer.Stop()
		c.reconnectTimer = nil
	}
	conn := c.conn
	c.conn = nil
	changed := c.state != "disconnected"
	c.state = "disconnected"
	waiting := c.takePendingLocked()
	onState := c.onState
	c.mu.Unlock()
	if conn != nil {
		conn.Close(websocket.StatusGoingAway, "")
	}
	for _, ch := range waiting {
		ch <- reply{err: reason}
	}
	if changed && onState != nil {
		onState("disconnected")
	}
}

func (c *cliClient) takePendingLocked() []chan reply {
	waiting := make([]chan reply, 0, len(c.pending))
	for id, ch := range c.pending {
		waiting = append(waiting, ch)
		delete(c.pending, id)
	}
	return waiting
}

// scheduleReconnect waits from 400 ms, growing to 5 s, before the next try.
func (c *cliClient) scheduleReconnect() {
	c.mu.Lock()
	defer c.mu.Unlock()
	if !c.wants || c.conn != nil {
		return
	}
	delay := c.reconnectDelay
	c.reconnectDelay = min(time.Duration(float64(c.reconnectDelay)*1.6), 5*time.Second)
	if c.reconnectTimer != nil {
		c.reconnectTimer.Stop()
	}
	c.reconnectTimer = time.AfterFunc(delay, func() {
		c.mu.Lock()
		due := c.wants && c.conn == nil
		c.reconnectTimer = nil
		c.mu.Unlock()
		if due {
			c.requestReconnect()
		}
	})
}

func (c *cliClient) requestReconnect() {
	if c.onReconnectNeeded != nil {
		c.onReconnectNeeded()
	} else {
		c.open()
	}
}

// request sends `{ id, method, params }` and returns the JSON-encoded `result`.
func (c *cliClient) request(ctx context.Context, method string, params jsontext.Value) (jsontext.Value, error) {
	if len(params) == 0 || string(params) == "null" {
		params = jsontext.Value("{}")
	}
	c.mu.Lock()
	conn := c.conn
	if conn == nil || c.state == "disconnected" {
		c.mu.Unlock()
		return nil, errNotRunning
	}
	id := c.nextID
	c.nextID++
	waiting := make(chan reply, 1)
	c.pending[id] = waiting
	c.mu.Unlock()

	frame, err := json.Marshal(struct {
		ID     int64          `json:"id"`
		Method string         `json:"method"`
		Params jsontext.Value `json:"params"`
	}{id, method, params})
	if err == nil {
		err = conn.Write(ctx, websocket.MessageText, frame)
	}
	if err != nil {
		c.mu.Lock()
		delete(c.pending, id)
		c.mu.Unlock()
		return nil, err
	}
	select {
	case answer := <-waiting:
		return answer.result, answer.err
	case <-ctx.Done():
		c.mu.Lock()
		delete(c.pending, id)
		c.mu.Unlock()
		return nil, ctx.Err()
	}
}
