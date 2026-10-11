// Package main embeds RelayKit; transport, authentication and cancellation stay in Rust.
package main

/*
#include <stdlib.h>
#include <stdint.h>
*/
import "C"

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"runtime/cgo"
	"strings"
	"unsafe"

	"github.com/QuantumNous/new-api/relaykit/dto"
	"github.com/QuantumNous/new-api/relaykit/relayconvert"
	"github.com/QuantumNous/new-api/relaykit/relayconvert/convmeta"
	"github.com/QuantumNous/new-api/relaykit/relayconvert/kitutil"
	"github.com/QuantumNous/new-api/relaykit/types"
)

type session struct {
	from, to                                            types.RelayFormat
	meta                                                *convmeta.Values
	stream                                              *relayconvert.ResponseStreamState
	terminal, finishSeen, finalized, usageSeen          bool
	inputKnown, outputKnown, cacheKnown, reasoningKnown bool
	cacheCreationKnown                                  bool
}

type command struct {
	Op    string            `json:"op"`
	From  types.RelayFormat `json:"from"`
	To    types.RelayFormat `json:"to"`
	Body  json.RawMessage   `json:"body"`
	Limit uint32            `json:"limit"`
	Model string            `json:"model"`
	Event string            `json:"event"`
	Data  string            `json:"data"`
}

type note struct {
	Code  string `json:"code"`
	Field string `json:"field"`
}
type reply struct {
	Handle uint64 `json:"handle,omitempty"`
	Body   any    `json:"body,omitempty"`
	Events []any  `json:"events,omitempty"`
	Notes  []note `json:"notes,omitempty"`
	Error  string `json:"error,omitempty"`
	Done   bool   `json:"done,omitempty"`
}

func init() {
	// RelayKit's default anomaly logging can contain payloads; only structured codes leave this bridge.
	discard := func(string) {}
	kitutil.SetLogging(discard, discard)
	kitutil.SetSystemErrorLogging(discard)
	relayconvert.SetMediaResolver(relayconvert.MediaResolver{
		GetBase64Data: func(_ context.Context, source types.FileSource, _ ...string) (string, string, error) {
			if source.IsURL() {
				return "", "application/octet-stream", nil
			}
			if inline, ok := source.(*types.Base64Source); ok && !strings.HasPrefix(inline.Base64Data, "data:") {
				if _, err := base64.StdEncoding.DecodeString(inline.Base64Data); err != nil {
					return "", "", err
				}
				return inline.Base64Data, inline.MimeType, nil
			}
			mime, data, err := dataURL(source.GetRawData())
			return data, mime, err
		},
		DecodeBase64FileData: dataURL,
	})
}

// Remote media is deliberately not fetched; RelayKit reports its unsupported MIME as an omitted block.
func dataURL(value string) (string, string, error) {
	header, data, ok := strings.Cut(value, ",")
	if !ok || !strings.HasPrefix(header, "data:") || !strings.HasSuffix(header, ";base64") {
		return "", "", errors.New("invalid_data_url")
	}
	if _, err := base64.StdEncoding.DecodeString(data); err != nil {
		return "", "", err
	}
	return strings.TrimSuffix(strings.TrimPrefix(header, "data:"), ";base64"), data, nil
}
func main() {}
func notes(ds []types.ConversionDiagnostic) []note {
	result := make([]note, 0, len(ds))
	for _, d := range ds {
		result = append(result, note{d.Code, "relaykit"})
	}
	return result
}
func decode(format types.RelayFormat, data []byte, request, stream bool) (any, error) {
	var value any
	switch format {
	case types.RelayFormatOpenAI:
		if request {
			value = &dto.GeneralOpenAIRequest{}
		} else if stream {
			value = &dto.ChatCompletionsStreamResponse{}
		} else {
			value = &dto.OpenAITextResponse{}
		}
	case types.RelayFormatOpenAIResponses:
		if request {
			value = &dto.OpenAIResponsesRequest{}
		} else if stream {
			value = &dto.ResponsesStreamResponse{}
		} else {
			value = &dto.OpenAIResponsesResponse{}
		}
	case types.RelayFormatClaude:
		if request {
			value = &dto.ClaudeRequest{}
		} else {
			value = &dto.ClaudeResponse{}
		}
	default:
		return nil, errors.New("protocol")
	}
	if err := json.Unmarshal(data, value); err != nil {
		return nil, err
	}
	return value, nil
}
func object(data []byte) (map[string]any, error) {
	var value map[string]any
	if err := json.Unmarshal(data, &value); err != nil || value == nil {
		return nil, errors.New("object")
	}
	return value, nil
}
func child(value map[string]any, key string) map[string]any {
	result, _ := value[key].(map[string]any)
	return result
}
func (s *session) observe(value map[string]any, stream bool) error {
	if value["error"] != nil || value["type"] == "error" || value["type"] == "response.failed" || value["type"] == "response.error" {
		return errors.New("upstream_error")
	}
	switch s.to {
	case types.RelayFormatOpenAI:
		choices, _ := value["choices"].([]any)
		if !stream && len(choices) == 0 {
			return errors.New("choices")
		}
		for _, choice := range choices {
			item, _ := choice.(map[string]any)
			if !stream {
				if reason, ok := item["finish_reason"].(string); !ok || reason == "" {
					return errors.New("finish_reason")
				}
			}
			if reason, ok := item["finish_reason"].(string); ok && reason != "" {
				s.finishSeen = true
			}
		}
	case types.RelayFormatOpenAIResponses:
		response := value
		if stream {
			response = child(value, "response")
		}
		if response != nil {
			status, _ := response["status"].(string)
			if status == "failed" || status == "cancelled" || (!stream && status != "completed" && status != "incomplete") {
				return errors.New("response_status")
			}
		}
		if kind := value["type"]; kind == "response.completed" || kind == "response.incomplete" || kind == "response.done" {
			if response == nil || response["error"] != nil || (response["status"] != "completed" && response["status"] != "incomplete") {
				return errors.New("response_status")
			}
			s.terminal = true
		}
	case types.RelayFormatClaude:
		if !stream {
			if reason, ok := value["stop_reason"].(string); !ok || reason == "" {
				return errors.New("stop_reason")
			}
		}
		if reason, ok := child(value, "delta")["stop_reason"].(string); ok && reason != "" {
			s.finishSeen = true
		}
		if value["type"] == "message_stop" {
			if !s.finishSeen {
				return errors.New("truncated")
			}
			s.terminal = true
		}
	}
	for _, usage := range []map[string]any{child(value, "usage"), child(child(value, "response"), "usage"), child(child(value, "message"), "usage")} {
		s.usageSeen = s.usageSeen || usage != nil
		input, output, cache, reasoning := "input_tokens", "output_tokens", "input_tokens_details", "output_tokens_details"
		if s.to == types.RelayFormatOpenAI {
			input, output, cache, reasoning = "prompt_tokens", "completion_tokens", "prompt_tokens_details", "completion_tokens_details"
		}
		s.inputKnown = s.inputKnown || usage[input] != nil
		s.outputKnown = s.outputKnown || usage[output] != nil
		s.cacheKnown = s.cacheKnown || child(usage, cache)["cached_tokens"] != nil || usage["cache_read_input_tokens"] != nil
		s.reasoningKnown = s.reasoningKnown || child(usage, reasoning)["reasoning_tokens"] != nil
		s.cacheCreationKnown = s.cacheCreationKnown || usage["cache_creation_input_tokens"] != nil || usage["claude_cache_creation_5_m_tokens"] != nil || usage["claude_cache_creation_1_h_tokens"] != nil
	}
	return nil
}
func (s *session) output(value any) (any, error) {
	if event, ok := value.(relayconvert.ChatToResponsesStreamEvent); ok {
		value = event.Payload
	}
	data, err := json.Marshal(value)
	if err != nil {
		return nil, err
	}
	body, err := object(data)
	if err != nil {
		return nil, err
	}
	// DTO zero values are not evidence that upstream reported those counts.
	for _, owner := range []map[string]any{body, child(body, "response"), child(body, "message")} {
		usage := child(owner, "usage")
		// RelayKit's DTO carries host accounting metadata and fields from every protocol.
		for _, key := range []string{"billing_usage", "usage_source", "usage_semantic", "claude_cache_creation_1_h_tokens", "claude_cache_creation_5_m_tokens"} {
			delete(usage, key)
		}
		if s.from == types.RelayFormatOpenAI {
			for _, key := range []string{"input_tokens", "output_tokens", "input_tokens_details", "output_tokens_details"} {
				delete(usage, key)
			}
		} else {
			for _, key := range []string{"prompt_tokens", "completion_tokens", "prompt_tokens_details", "completion_tokens_details"} {
				delete(usage, key)
			}
		}
		if s.from == types.RelayFormatClaude {
			if !s.cacheKnown {
				delete(usage, "cache_read_input_tokens")
			}
			if !s.cacheCreationKnown {
				delete(usage, "cache_creation_input_tokens")
				delete(usage, "cache_creation")
			}
		}
		if owner == nil || s.from == types.RelayFormatClaude {
			continue
		}
		if !s.usageSeen {
			if _, ok := owner["usage"]; ok {
				owner["usage"] = nil
			}
			continue
		}
		input, output, cache, reasoning := "input_tokens", "output_tokens", "input_tokens_details", "output_tokens_details"
		if s.from == types.RelayFormatOpenAI {
			input, output, cache, reasoning = "prompt_tokens", "completion_tokens", "prompt_tokens_details", "completion_tokens_details"
		}
		if !s.inputKnown {
			delete(usage, input)
		}
		if !s.outputKnown {
			delete(usage, output)
		}
		if !s.inputKnown || !s.outputKnown {
			delete(usage, "total_tokens")
		}
		if !s.cacheKnown {
			delete(usage, cache)
		}
		if !s.reasoningKnown {
			delete(usage, reasoning)
		}
	}
	return body, nil
}
func (s *session) events(results []relayconvert.ResponseResult) (reply, error) {
	out := reply{}
	for _, result := range results {
		if result.Value == nil {
			continue
		}
		value, err := s.output(result.Value)
		if err != nil {
			return reply{}, err
		}
		out.Events = append(out.Events, value)
	}
	out.Notes = notes(s.stream.Diagnostics())
	if !s.inputKnown || !s.outputKnown {
		out.Notes = append(out.Notes, note{"usage_unreported", "usage"})
	}
	return out, nil
}
func run(handle C.uintptr_t, cmd command) (reply, error) {
	ctx := context.Background()
	if cmd.Op == "prepare" {
		body, err := object(cmd.Body)
		if err != nil {
			return reply{}, err
		}
		compatibilityNotes := []note{{"best_effort_projection", "request.extensions"}}
		// Stateful references cannot be reconstructed across independent providers.
		if cmd.From == types.RelayFormatOpenAIResponses {
			for _, key := range []string{"conversation", "previous_response_id", "prompt", "context_management"} {
				if body[key] != nil {
					delete(body, key)
					compatibilityNotes = append(compatibilityNotes, note{"state_reference_omitted", key})
				}
			}
		}
		if cmd.From == types.RelayFormatOpenAI {
			if count, ok := body["n"].(float64); ok && count > 1 {
				body["n"] = 1
				compatibilityNotes = append(compatibilityNotes, note{"first_choice_only", "n"})
			}
		}
		cmd.Body, err = json.Marshal(body)
		if err != nil {
			return reply{}, err
		}
		model, _ := body["model"].(string)
		s := &session{from: cmd.From, to: cmd.To, meta: &convmeta.Values{OriginModelName: model, UpstreamModelName: model, ChannelMetaAttached: true, Options: &convmeta.Options{}}}
		if cmd.Limit > 0 {
			s.meta.Options.Claude.DefaultMaxTokens = func(string) int { return int(cmd.Limit) }
		}
		request, err := decode(cmd.From, cmd.Body, true, false)
		if err != nil {
			return reply{}, err
		}
		result, err := relayconvert.ConvertRequest(ctx, s.meta, cmd.To, request)
		if err != nil {
			return reply{}, err
		}
		return reply{Handle: uint64(cgo.NewHandle(s)), Body: result.Value, Notes: append(compatibilityNotes, notes(result.Diagnostics)...)}, nil
	}
	s := cgo.Handle(handle).Value().(*session)
	switch cmd.Op {
	case "response":
		value, err := object(cmd.Body)
		if err != nil {
			return reply{}, err
		}
		if err := s.observe(value, false); err != nil {
			return reply{}, err
		}
		response, err := decode(s.to, cmd.Body, false, false)
		if err != nil {
			return reply{}, err
		}
		result, err := relayconvert.ConvertResponse(ctx, s.meta, s.from, response)
		if err != nil {
			return reply{}, err
		}
		output, err := s.output(result.Value)
		diagnostics := notes(result.Diagnostics)
		if !s.inputKnown || !s.outputKnown {
			diagnostics = append(diagnostics, note{"usage_unreported", "usage"})
		}
		return reply{Body: output, Notes: diagnostics}, err
	case "stream":
		var err error
		s.stream, err = relayconvert.NewResponseStreamState(s.to, s.from, relayconvert.ResponseStreamOptions{Model: cmd.Model, IncludeUsage: true, EmitSequenceNumber: true})
		return reply{}, err
	case "push":
		if s.terminal || s.finalized || strings.TrimSpace(cmd.Data) == "" {
			return reply{}, nil
		}
		if strings.TrimSpace(cmd.Data) == "[DONE]" {
			if s.to != types.RelayFormatOpenAI {
				return reply{}, errors.New("unexpected_done")
			}
			if !s.finishSeen {
				return reply{}, errors.New("truncated")
			}
			s.terminal = true
			return reply{}, nil
		}
		value, err := object([]byte(cmd.Data))
		if err != nil {
			return reply{}, err
		}
		if cmd.Event == "error" {
			return reply{}, errors.New("upstream_error")
		}
		if value["type"] == nil && cmd.Event != "" && s.to != types.RelayFormatOpenAI {
			value["type"] = cmd.Event
		}
		if err := s.observe(value, true); err != nil {
			return reply{}, err
		}
		data, err := json.Marshal(value)
		if err != nil {
			return reply{}, err
		}
		response, err := decode(s.to, data, false, true)
		if err != nil {
			return reply{}, err
		}
		results, err := relayconvert.ConvertStreamResponseChunk(ctx, s.meta, s.stream, response)
		if err != nil {
			return reply{}, err
		}
		return s.events(results)
	case "finish":
		if s.finalized {
			return reply{}, nil
		}
		if !s.terminal && !(s.to == types.RelayFormatOpenAI && s.finishSeen) {
			return reply{}, errors.New("truncated")
		}
		results, err := relayconvert.FinalizeStreamResponse(ctx, s.meta, s.stream)
		if err != nil {
			return reply{}, err
		}
		s.finalized = true
		result, err := s.events(results)
		result.Done = s.from == types.RelayFormatOpenAI
		return result, err
	}
	return reply{}, errors.New("operation")
}

//export velune_relaykit_call
func velune_relaykit_call(handle C.uintptr_t, data *C.char, size C.size_t) (output *C.char) {
	defer func() {
		if recover() != nil {
			output = C.CString(`{"error":"relaykit_panic"}`)
		}
	}()
	var cmd command
	if uint64(size) > uint64(^uint32(0)>>1) || json.Unmarshal(C.GoBytes(unsafe.Pointer(data), C.int(size)), &cmd) != nil {
		return C.CString(`{"error":"bridge_input"}`)
	}
	result, err := run(handle, cmd)
	if err != nil {
		// Never export upstream errors: they can interpolate request bodies.
		code := "relaykit_conversion"
		if err.Error() == "truncated" {
			code = "truncated"
		}
		result = reply{Error: code}
	}
	encoded, err := json.Marshal(result)
	if err != nil {
		return C.CString(`{"error":"bridge_output"}`)
	}
	return C.CString(string(encoded))
}

//export velune_relaykit_release
func velune_relaykit_release(handle C.uintptr_t) { cgo.Handle(handle).Delete() }

//export velune_relaykit_free
func velune_relaykit_free(data *C.char) { C.free(unsafe.Pointer(data)) }
