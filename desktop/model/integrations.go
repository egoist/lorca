package model

import (
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"slices"
	"strings"
)

func (s *Store) mockIntegrationState(pluginID, runnerID string, state PluginState, detail string, done func(error)) bool {
	if !s.IsMock {
		return false
	}
	if runner := s.Device(runnerID); runner != nil {
		for _, plugin := range runner.Plugins {
			if plugin.ID == pluginID && plugin.ServiceID != "" {
				s.post(func() {
					plugin.State, plugin.Detail = state, detail
					s.rememberPlugin(runnerID, plugin)
					if done != nil {
						done(nil)
					}
				})
				return true
			}
		}
	}
	return false
}

// accounts are a service's named accounts on a Runner.
func (s *Store) accounts(serviceID, runnerID string) []InstalledPlugin {
	var accounts []InstalledPlugin
	if runner := s.Device(runnerID); runner != nil {
		for _, plugin := range runner.Plugins {
			if plugin.ServiceID == serviceID && !plugin.IsMcpServer() {
				accounts = append(accounts, plugin)
			}
		}
	}
	return accounts
}

// rememberPlugin publishes a management reply before the next encrypted Runner advertisement.
// It is called on the store's ordered main-thread reply queue.
func (s *Store) rememberPlugin(runnerID string, plugin InstalledPlugin) {
	if runner := s.Device(runnerID); runner != nil && plugin.ID != "" {
		if i := slices.IndexFunc(runner.Plugins, func(p InstalledPlugin) bool { return p.ID == plugin.ID }); i >= 0 {
			runner.Plugins[i] = plugin
		} else {
			runner.Plugins = append(runner.Plugins, plugin)
		}
		s.emit(Event{Kind: EventRosterChanged})
	}
}

// InstallPluginAccount adds a named account of a marketplace service on a Runner. A blank name
// becomes the next free "Account 1". Tokens and client settings stay with the Runner's CLI.
func (s *Store) InstallPluginAccount(serviceID, runnerID, accountName string, done func(InstalledPlugin, error)) {
	params := map[string]any{"runner_id": runnerID, "plugin_id": serviceID, "account_name": accountName}
	if s.IsMock {
		s.post(func() {
			taken := func(name string) bool {
				return slices.ContainsFunc(s.accounts(serviceID, runnerID), func(account InstalledPlugin) bool { return strings.EqualFold(account.AccountName, name) })
			}
			name := strings.TrimSpace(accountName)
			for n := 1; name == ""; n++ {
				if candidate := fmt.Sprintf("Account %d", n); !taken(candidate) {
					name = candidate
				}
			}
			if taken(name) {
				done(InstalledPlugin{}, fmt.Errorf("There is already an account named %s.", name))
				return
			}
			var id [16]byte
			if _, err := rand.Read(id[:]); err != nil {
				done(InstalledPlugin{}, err)
				return
			}
			serviceName := serviceID
			for _, manifest := range mockMarketplace().Plugins {
				if manifest.ID == serviceID {
					serviceName = manifest.Name
				}
			}
			plugin := InstalledPlugin{ID: serviceID + "-" + hex.EncodeToString(id[:]), ServiceID: serviceID, AccountName: name,
				Name: serviceName + " · " + name, State: PluginNeedsAuth, Detail: "Sign in"}
			s.rememberPlugin(runnerID, plugin)
			done(plugin, nil)
		})
		return
	}
	Async(s, func() (InstalledPlugin, error) {
		reply, err := call[pluginReply](s, "plugins.install", params)
		return ToPlugin(reply.Status), err
	}, func(plugin InstalledPlugin, err error) {
		if err == nil {
			s.rememberPlugin(runnerID, plugin)
		}
		done(plugin, err)
	})
}

// RenamePluginAccount renames a named account; its id, which tools and rules use, stays.
func (s *Store) RenamePluginAccount(pluginID, runnerID, accountName string, done func(InstalledPlugin, error)) {
	params := map[string]any{"runner_id": runnerID, "plugin_id": pluginID, "account_name": accountName}
	if s.IsMock {
		s.post(func() {
			runner := s.Device(runnerID)
			name := strings.TrimSpace(accountName)
			if runner == nil || name == "" {
				done(InstalledPlugin{}, fmt.Errorf("Give the account a name."))
				return
			}
			for _, plugin := range runner.Plugins {
				if plugin.ID == pluginID && plugin.ServiceID != "" {
					for _, other := range s.accounts(plugin.ServiceID, runnerID) {
						if other.ID != pluginID && strings.EqualFold(other.AccountName, name) {
							done(InstalledPlugin{}, fmt.Errorf("There is already an account named %s.", name))
							return
						}
					}
					plugin.Name = strings.SplitN(plugin.Name, " · ", 2)[0] + " · " + name
					plugin.AccountName = name
					s.rememberPlugin(runnerID, plugin)
					done(plugin, nil)
					return
				}
			}
			done(InstalledPlugin{}, fmt.Errorf("Unknown named account."))
		})
		return
	}
	Async(s, func() (InstalledPlugin, error) {
		reply, err := call[pluginReply](s, "plugins.rename", params)
		return ToPlugin(reply.Status), err
	}, func(plugin InstalledPlugin, err error) {
		if err == nil {
			s.rememberPlugin(runnerID, plugin)
		}
		done(plugin, err)
	})
}
