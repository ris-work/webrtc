//! Pure Rust implementation of the constraint logic defined in the ["Media Capture and Streams"][mediacapture_streams] spec.
//!
//! [mediacapture_streams]: https://www.w3.org/TR/mediacapture-streams/

pub mod algorithms;
pub mod errors;
pub mod macros;
pub mod property;

mod capabilities;
mod capability;
mod constraint;
mod constraints;
mod enumerations;
mod setting;
mod settings;
mod supported_constraints;

#[allow(unused_imports)]
pub use self::{
    capabilities::MediaTrackCapabilities,
    capability::MediaTrackCapability,
    constraint::{
        BareOrMediaTrackConstraint, BareOrValueConstraint, BareOrValueRangeConstraint,
        BareOrValueSequenceConstraint, MediaTrackConstraintResolutionStrategy,
        ResolvedMediaTrackConstraint, ResolvedValueConstraint, ResolvedValueRangeConstraint,
        ResolvedValueSequenceConstraint, SanitizedMediaTrackConstraint,
    },
    constraints::{
        BareOrAdvancedMediaTrackConstraints, BareOrBoolOrMediaTrackConstraints,
        BareOrMandatoryMediaTrackConstraints, BareOrMediaStreamConstraints,
        BareOrMediaTrackConstraintSet, BareOrMediaTrackConstraints, BoolOrMediaTrackConstraints,
        ResolvedAdvancedMediaTrackConstraints, ResolvedMandatoryMediaTrackConstraints,
        ResolvedMediaStreamConstraints, ResolvedMediaTrackConstraintSet,
        ResolvedMediaTrackConstraints, SanitizedMandatoryMediaTrackConstraints,
        SanitizedMediaTrackConstraintSet, SanitizedMediaTrackConstraints,
    },
    enumerations::{FacingMode, ResizeMode},
    setting::MediaTrackSetting,
    settings::MediaTrackSettings,
    supported_constraints::MediaTrackSupportedConstraints,
};

#[allow(unused_imports)]
pub(crate) use self::{capabilities::MediaStreamCapabilities, settings::MediaStreamSettings};
