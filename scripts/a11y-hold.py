#!/usr/bin/python3 -I
"""Keep an AT client connected to the accessibility bus, so at-spi-bus-launcher keeps
org.a11y.Status.IsEnabled = true and apps (accesskit) stay connected: window ids stay stable."""
import gi, signal, sys
gi.require_version("Atspi", "2.0")
from gi.repository import Atspi
Atspi.get_desktop(0)
l = Atspi.EventListener.new(lambda e: None)
l.register("window:activate")
signal.signal(signal.SIGTERM, lambda *a: (Atspi.event_quit(), sys.exit(0)))
Atspi.event_main()
