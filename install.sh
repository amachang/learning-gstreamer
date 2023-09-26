#!/bin/bash

set -eux

brew install gstreamer
cargo add gstreamer-video gstreamer-audio gstreamer-app

