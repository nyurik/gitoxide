use std::{borrow::Cow, collections::HashMap, convert::TryInto};

use bstr::BStr;
use smallvec::ToSmallVec;

use crate::{
    file::{mutable::multi_value::EntryData, Index, MetadataFilter, MultiValueMut, Size, ValueMut},
    lookup,
    parse::{section, Event},
    File, Key,
};

/// # Raw value API
///
/// These functions are the raw value API, returning normalized byte strings.
impl<'event> File<'event> {
    /// TODO
    pub fn raw_value(&self, key: impl Key) -> Result<Cow<'_, BStr>, lookup::existing::Error> {
        self.raw_value_by(key.section_name(), key.subsection_name(), key.name())
    }

    /// Returns an uninterpreted value given a section, an optional subsection
    /// and key.
    ///
    /// Consider [`Self::raw_values()`] if you want to get all values of
    /// a multivar instead.
    pub fn raw_value_by(
        &self,
        section_name: impl AsRef<str>,
        subsection_name: Option<&BStr>,
        key: impl AsRef<str>,
    ) -> Result<Cow<'_, BStr>, lookup::existing::Error> {
        self.raw_value_filter(section_name, subsection_name, key, &mut |_| true)
    }

    /// Returns an uninterpreted value given a section, an optional subsection
    /// and key, if it passes the `filter`.
    ///
    /// Consider [`Self::raw_values()`] if you want to get all values of
    /// a multivar instead.
    pub fn raw_value_filter(
        &self,
        section_name: impl AsRef<str>,
        subsection_name: Option<&BStr>,
        key: impl AsRef<str>,
        filter: &mut MetadataFilter,
    ) -> Result<Cow<'_, BStr>, lookup::existing::Error> {
        self.raw_value_filter_inner(section_name.as_ref(), subsection_name, key.as_ref(), filter)
    }

    fn raw_value_filter_inner(
        &self,
        section_name: &str,
        subsection_name: Option<&BStr>,
        key: &str,
        filter: &mut MetadataFilter,
    ) -> Result<Cow<'_, BStr>, lookup::existing::Error> {
        let section_ids = self.section_ids_by_name_and_subname(section_name, subsection_name)?;
        for section_id in section_ids.rev() {
            let section = self.sections.get(&section_id).expect("known section id");
            if !filter(section.meta()) {
                continue;
            }
            if let Some(v) = section.value(key) {
                return Ok(v);
            }
        }

        Err(lookup::existing::Error::KeyMissing)
    }

    /// Returns a mutable reference to an uninterpreted value given a section,
    /// an optional subsection and key.
    ///
    /// Consider [`Self::raw_values_mut`] if you want to get mutable
    /// references to all values of a multivar instead.
    pub fn raw_value_mut<'lookup>(
        &mut self,
        section_name: impl AsRef<str>,
        subsection_name: Option<&'lookup BStr>,
        key: &'lookup str,
    ) -> Result<ValueMut<'_, 'lookup, 'event>, lookup::existing::Error> {
        self.raw_value_mut_filter(section_name, subsection_name, key, &mut |_| true)
    }

    /// Returns a mutable reference to an uninterpreted value given a section,
    /// an optional subsection and key, and if it passes `filter`.
    ///
    /// Consider [`Self::raw_values_mut`] if you want to get mutable
    /// references to all values of a multivar instead.
    pub fn raw_value_mut_filter<'lookup>(
        &mut self,
        section_name: impl AsRef<str>,
        subsection_name: Option<&'lookup BStr>,
        key: &'lookup str,
        filter: &mut MetadataFilter,
    ) -> Result<ValueMut<'_, 'lookup, 'event>, lookup::existing::Error> {
        self.raw_value_mut_filter_inner(section_name.as_ref(), subsection_name, key, filter)
    }

    fn raw_value_mut_filter_inner<'lookup>(
        &mut self,
        section_name: &str,
        subsection_name: Option<&'lookup BStr>,
        key: &'lookup str,
        filter: &mut MetadataFilter,
    ) -> Result<ValueMut<'_, 'lookup, 'event>, lookup::existing::Error> {
        let mut section_ids = self
            .section_ids_by_name_and_subname(section_name, subsection_name)?
            .rev();
        let key = section::Key(Cow::<BStr>::Borrowed(key.into()));

        while let Some(section_id) = section_ids.next() {
            let mut index = 0;
            let mut size = 0;
            let mut found_key = false;
            let section = self.sections.get(&section_id).expect("known section id");
            if !filter(section.meta()) {
                continue;
            }
            for (i, event) in section.as_ref().iter().enumerate() {
                match event {
                    Event::SectionKey(event_key) if *event_key == key => {
                        found_key = true;
                        index = i;
                        size = 1;
                    }
                    Event::Newline(_) | Event::Whitespace(_) | Event::ValueNotDone(_) if found_key => {
                        size += 1;
                    }
                    Event::ValueDone(_) | Event::Value(_) if found_key => {
                        found_key = false;
                        size += 1;
                    }
                    Event::KeyValueSeparator if found_key => {
                        size += 1;
                    }
                    _ => {}
                }
            }

            if size == 0 {
                continue;
            }

            drop(section_ids);
            let nl = self.detect_newline_style().to_smallvec();
            return Ok(ValueMut {
                section: self.sections.get_mut(&section_id).expect("known section-id").to_mut(nl),
                key,
                index: Index(index),
                size: Size(size),
            });
        }

        Err(lookup::existing::Error::KeyMissing)
    }

    /// TODO
    pub fn raw_values(&self, key: impl Key) -> Result<Vec<Cow<'_, BStr>>, lookup::existing::Error> {
        self.raw_values_by(key.section_name(), key.subsection_name(), key.name())
    }

    /// Returns all uninterpreted values given a section, an optional subsection
    /// ain order of occurrence.
    ///
    /// The ordering means that the last of the returned values is the one that would be the
    /// value used in the single-value case.and key.
    ///
    /// # Examples
    ///
    /// If you have the following config:
    ///
    /// ```text
    /// [core]
    ///     a = b
    /// [core]
    ///     a = c
    ///     a = d
    /// ```
    ///
    /// Attempting to get all values of `a` yields the following:
    ///
    /// ```
    /// # use gix_config::File;
    /// # use std::borrow::Cow;
    /// # use std::convert::TryFrom;
    /// # use bstr::BStr;
    /// # let git_config = gix_config::File::try_from("[core]a=b\n[core]\na=c\na=d").unwrap();
    /// assert_eq!(
    ///     git_config.raw_values("core", None, "a").unwrap(),
    ///     vec![
    ///         Cow::<BStr>::Borrowed("b".into()),
    ///         Cow::<BStr>::Borrowed("c".into()),
    ///         Cow::<BStr>::Borrowed("d".into()),
    ///     ],
    /// );
    /// ```
    ///
    /// Consider [`Self::raw_value`] if you want to get the resolved single
    /// value for a given key, if your key does not support multi-valued values.
    pub fn raw_values_by(
        &self,
        section_name: impl AsRef<str>,
        subsection_name: Option<&BStr>,
        key: impl AsRef<str>,
    ) -> Result<Vec<Cow<'_, BStr>>, lookup::existing::Error> {
        self.raw_values_filter(section_name, subsection_name, key, &mut |_| true)
    }

    /// Returns all uninterpreted values given a section, an optional subsection
    /// and key, if the value passes `filter`, in order of occurrence.
    ///
    /// The ordering means that the last of the returned values is the one that would be the
    /// value used in the single-value case.
    pub fn raw_values_filter(
        &self,
        section_name: impl AsRef<str>,
        subsection_name: Option<&BStr>,
        key: impl AsRef<str>,
        filter: &mut MetadataFilter,
    ) -> Result<Vec<Cow<'_, BStr>>, lookup::existing::Error> {
        self.raw_values_filter_inner(section_name.as_ref(), subsection_name, key.as_ref(), filter)
    }

    fn raw_values_filter_inner(
        &self,
        section_name: &str,
        subsection_name: Option<&BStr>,
        key: &str,
        filter: &mut MetadataFilter,
    ) -> Result<Vec<Cow<'_, BStr>>, lookup::existing::Error> {
        let mut values = Vec::new();
        let section_ids = self.section_ids_by_name_and_subname(section_name, subsection_name)?;
        for section_id in section_ids {
            let section = self.sections.get(&section_id).expect("known section id");
            if !filter(section.meta()) {
                continue;
            }
            values.extend(section.values(key));
        }

        if values.is_empty() {
            Err(lookup::existing::Error::KeyMissing)
        } else {
            Ok(values)
        }
    }

    /// TODO
    pub fn raw_values_mut(
        &mut self,
        key: &'event dyn Key,
    ) -> Result<MultiValueMut<'_, '_, 'event>, lookup::existing::Error> {
        self.raw_values_mut_by(key.section_name(), key.subsection_name(), key.name())
    }

    /// Returns mutable references to all uninterpreted values given a section,
    /// an optional subsection and key.
    ///
    /// # Examples
    ///
    /// If you have the following config:
    ///
    /// ```text
    /// [core]
    ///     a = b
    /// [core]
    ///     a = c
    ///     a = d
    /// ```
    ///
    /// Attempting to get all values of `a` yields the following:
    ///
    /// ```
    /// # use gix_config::File;
    /// # use std::borrow::Cow;
    /// # use std::convert::TryFrom;
    /// # use bstr::BStr;
    /// # let mut git_config = gix_config::File::try_from("[core]a=b\n[core]\na=c\na=d").unwrap();
    /// assert_eq!(
    ///     git_config.raw_values("core", None, "a")?,
    ///     vec![
    ///         Cow::<BStr>::Borrowed("b".into()),
    ///         Cow::<BStr>::Borrowed("c".into()),
    ///         Cow::<BStr>::Borrowed("d".into())
    ///     ]
    /// );
    ///
    /// git_config.raw_values_mut("core", None, "a")?.set_all("g");
    ///
    /// assert_eq!(
    ///     git_config.raw_values("core", None, "a")?,
    ///     vec![
    ///         Cow::<BStr>::Borrowed("g".into()),
    ///         Cow::<BStr>::Borrowed("g".into()),
    ///         Cow::<BStr>::Borrowed("g".into())
    ///     ],
    /// );
    /// # Ok::<(), gix_config::lookup::existing::Error>(())
    /// ```
    ///
    /// Consider [`Self::raw_value`] if you want to get the resolved single
    /// value for a given key, if your key does not support multi-valued values.
    ///
    /// Note that this operation is relatively expensive, requiring a full
    /// traversal of the config.
    pub fn raw_values_mut_by<'lookup>(
        &mut self,
        section_name: impl AsRef<str>,
        subsection_name: Option<&'lookup BStr>,
        key: &'lookup str,
    ) -> Result<MultiValueMut<'_, 'lookup, 'event>, lookup::existing::Error> {
        self.raw_values_mut_filter(section_name, subsection_name, key, &mut |_| true)
    }

    /// Returns mutable references to all uninterpreted values given a section,
    /// an optional subsection and key, if their sections pass `filter`.
    pub fn raw_values_mut_filter<'lookup>(
        &mut self,
        section_name: impl AsRef<str>,
        subsection_name: Option<&'lookup BStr>,
        key: &'lookup str,
        filter: &mut MetadataFilter,
    ) -> Result<MultiValueMut<'_, 'lookup, 'event>, lookup::existing::Error> {
        self.raw_values_mut_filter_inner(section_name.as_ref(), subsection_name, key, filter)
    }

    fn raw_values_mut_filter_inner<'lookup>(
        &mut self,
        section_name: &str,
        subsection_name: Option<&'lookup BStr>,
        key: &'lookup str,
        filter: &mut MetadataFilter,
    ) -> Result<MultiValueMut<'_, 'lookup, 'event>, lookup::existing::Error> {
        let section_ids = self.section_ids_by_name_and_subname(section_name, subsection_name)?;
        let key = section::Key(Cow::<BStr>::Borrowed(key.into()));

        let mut offsets = HashMap::new();
        let mut entries = Vec::new();
        for section_id in section_ids.rev() {
            let mut last_boundary = 0;
            let mut expect_value = false;
            let mut offset_list = Vec::new();
            let mut offset_index = 0;
            let section = self.sections.get(&section_id).expect("known section-id");
            if !filter(section.meta()) {
                continue;
            }
            for (i, event) in section.as_ref().iter().enumerate() {
                match event {
                    Event::SectionKey(event_key) if *event_key == key => {
                        expect_value = true;
                        offset_list.push(i - last_boundary);
                        offset_index += 1;
                        last_boundary = i;
                    }
                    Event::Value(_) | Event::ValueDone(_) if expect_value => {
                        expect_value = false;
                        entries.push(EntryData {
                            section_id,
                            offset_index,
                        });
                        offset_list.push(i - last_boundary + 1);
                        offset_index += 1;
                        last_boundary = i + 1;
                    }
                    _ => (),
                }
            }
            offsets.insert(section_id, offset_list);
        }

        entries.sort();

        if entries.is_empty() {
            Err(lookup::existing::Error::KeyMissing)
        } else {
            Ok(MultiValueMut {
                section: &mut self.sections,
                key,
                indices_and_sizes: entries,
                offsets,
            })
        }
    }

    /// Sets a value in a given `section_name`, optional `subsection_name`, and `key`.
    /// Note sections named `section_name` and `subsection_name` (if not `None`)
    /// must exist for this method to work.
    ///
    /// # Examples
    ///
    /// Given the config,
    ///
    /// ```text
    /// [core]
    ///     a = b
    /// [core]
    ///     a = c
    ///     a = d
    /// ```
    ///
    /// Setting a new value to the key `core.a` will yield the following:
    ///
    /// ```
    /// # use gix_config::File;
    /// # use std::borrow::Cow;
    /// # use bstr::BStr;
    /// # use std::convert::TryFrom;
    /// # let mut git_config = gix_config::File::try_from("[core]a=b\n[core]\na=c\na=d").unwrap();
    /// git_config.set_existing_raw_value("core", None, "a", "e")?;
    /// assert_eq!(git_config.raw_value("core", None, "a")?, Cow::<BStr>::Borrowed("e".into()));
    /// assert_eq!(
    ///     git_config.raw_values("core", None, "a")?,
    ///     vec![
    ///         Cow::<BStr>::Borrowed("b".into()),
    ///         Cow::<BStr>::Borrowed("c".into()),
    ///         Cow::<BStr>::Borrowed("e".into())
    ///     ],
    /// );
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn set_existing_raw_value<'b>(
        &mut self,
        section_name: impl AsRef<str>,
        subsection_name: Option<&BStr>,
        key: impl AsRef<str>,
        new_value: impl Into<&'b BStr>,
    ) -> Result<(), lookup::existing::Error> {
        self.raw_value_mut(section_name, subsection_name, key.as_ref())
            .map(|mut entry| entry.set(new_value))
    }

    /// Sets a value in a given `section_name`, optional `subsection_name`, and `key`.
    /// Creates the section if necessary and the key as well, or overwrites the last existing value otherwise.
    ///
    /// # Examples
    ///
    /// Given the config,
    ///
    /// ```text
    /// [core]
    ///     a = b
    /// ```
    ///
    /// Setting a new value to the key `core.a` will yield the following:
    ///
    /// ```
    /// # use gix_config::File;
    /// # use std::borrow::Cow;
    /// # use bstr::BStr;
    /// # use std::convert::TryFrom;
    /// # let mut git_config = gix_config::File::try_from("[core]a=b").unwrap();
    /// let prev = git_config.set_raw_value(&"core.a", "e")?;
    /// git_config.set_raw_value(&"core.b", "f")?;
    /// assert_eq!(prev.expect("present").as_ref(), "b");
    /// assert_eq!(git_config.raw_value("core", None, "a")?, Cow::<BStr>::Borrowed("e".into()));
    /// assert_eq!(git_config.raw_value("core", None, "b")?, Cow::<BStr>::Borrowed("f".into()));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn set_raw_value_by<'b, Key, E>(
        &mut self,
        section_name: impl AsRef<str>,
        subsection_name: Option<&BStr>,
        key: Key,
        new_value: impl Into<&'b BStr>,
    ) -> Result<Option<Cow<'event, BStr>>, crate::file::set_raw_value::Error>
    where
        Key: TryInto<section::Key<'event>, Error = E>,
        section::key::Error: From<E>,
    {
        self.set_raw_value_filter(section_name, subsection_name, key, new_value, &mut |_| true)
    }

    /// TODO
    pub fn set_raw_value<'b>(
        &mut self,
        key: &'event dyn Key,
        new_value: impl Into<&'b BStr>,
    ) -> Result<Option<Cow<'event, BStr>>, crate::file::set_raw_value::Error> {
        self.set_raw_value_by(key.section_name(), None, key.name(), new_value)
    }

    /// TODO
    pub fn set_raw_value_in_subsection<'b>(
        &mut self,
        key: &'event dyn Key,
        subsection_name: &BStr,
        new_value: impl Into<&'b BStr>,
    ) -> Result<Option<Cow<'event, BStr>>, crate::file::set_raw_value::Error> {
        self.set_raw_value_by(key.section_name(), Some(subsection_name), key.name(), new_value)
    }

    /// Similar to [`set_raw_value()`][Self::set_raw_value()], but only sets existing values in sections matching
    /// `filter`, creating a new section otherwise.
    pub fn set_raw_value_filter<'b, Key, E>(
        &mut self,
        section_name: impl AsRef<str>,
        subsection_name: Option<&BStr>,
        key: Key,
        new_value: impl Into<&'b BStr>,
        filter: &mut MetadataFilter,
    ) -> Result<Option<Cow<'event, BStr>>, crate::file::set_raw_value::Error>
    where
        Key: TryInto<section::Key<'event>, Error = E>,
        section::key::Error: From<E>,
    {
        let mut section = self.section_mut_or_create_new_filter(section_name, subsection_name, filter)?;
        Ok(section.set(key.try_into().map_err(section::key::Error::from)?, new_value.into()))
    }

    /// Sets a multivar in a given section, optional subsection, and key value.
    ///
    /// This internally zips together the new values and the existing values.
    /// As a result, if more new values are provided than the current amount of
    /// multivars, then the latter values are not applied. If there are less
    /// new values than old ones then the remaining old values are unmodified.
    ///
    /// **Note**: Mutation order is _not_ guaranteed and is non-deterministic.
    /// If you need finer control over which values of the multivar are set,
    /// consider using [`raw_values_mut()`][Self::raw_values_mut()], which will let you iterate
    /// and check over the values instead. This is best used as a convenience
    /// function for setting multivars whose values should be treated as an
    /// unordered set.
    ///
    /// # Examples
    ///
    /// Let us use the follow config for all examples:
    ///
    /// ```text
    /// [core]
    ///     a = b
    /// [core]
    ///     a = c
    ///     a = d
    /// ```
    ///
    /// Setting an equal number of values:
    ///
    /// ```
    /// # use gix_config::File;
    /// # use std::borrow::Cow;
    /// # use std::convert::TryFrom;
    /// # use bstr::BStr;
    /// # let mut git_config = gix_config::File::try_from("[core]a=b\n[core]\na=c\na=d").unwrap();
    /// let new_values = vec![
    ///     "x",
    ///     "y",
    ///     "z",
    /// ];
    /// git_config.set_existing_raw_multi_value("core", None, "a", new_values.into_iter())?;
    /// let fetched_config = git_config.raw_values("core", None, "a")?;
    /// assert!(fetched_config.contains(&Cow::<BStr>::Borrowed("x".into())));
    /// assert!(fetched_config.contains(&Cow::<BStr>::Borrowed("y".into())));
    /// assert!(fetched_config.contains(&Cow::<BStr>::Borrowed("z".into())));
    /// # Ok::<(), gix_config::lookup::existing::Error>(())
    /// ```
    ///
    /// Setting less than the number of present values sets the first ones found:
    ///
    /// ```
    /// # use gix_config::File;
    /// # use std::borrow::Cow;
    /// # use std::convert::TryFrom;
    /// # use bstr::BStr;
    /// # let mut git_config = gix_config::File::try_from("[core]a=b\n[core]\na=c\na=d").unwrap();
    /// let new_values = vec![
    ///     "x",
    ///     "y",
    /// ];
    /// git_config.set_existing_raw_multi_value("core", None, "a", new_values.into_iter())?;
    /// let fetched_config = git_config.raw_values("core", None, "a")?;
    /// assert!(fetched_config.contains(&Cow::<BStr>::Borrowed("x".into())));
    /// assert!(fetched_config.contains(&Cow::<BStr>::Borrowed("y".into())));
    /// # Ok::<(), gix_config::lookup::existing::Error>(())
    /// ```
    ///
    /// Setting more than the number of present values discards the rest:
    ///
    /// ```
    /// # use gix_config::File;
    /// # use std::borrow::Cow;
    /// # use std::convert::TryFrom;
    /// # use bstr::BStr;
    /// # let mut git_config = gix_config::File::try_from("[core]a=b\n[core]\na=c\na=d").unwrap();
    /// let new_values = vec![
    ///     "x",
    ///     "y",
    ///     "z",
    ///     "discarded",
    /// ];
    /// git_config.set_existing_raw_multi_value("core", None, "a", new_values)?;
    /// assert!(!git_config.raw_values("core", None, "a")?.contains(&Cow::<BStr>::Borrowed("discarded".into())));
    /// # Ok::<(), gix_config::lookup::existing::Error>(())
    /// ```
    pub fn set_existing_raw_multi_value<'a, Iter, Item>(
        &mut self,
        section_name: impl AsRef<str>,
        subsection_name: Option<&BStr>,
        key: impl AsRef<str>,
        new_values: Iter,
    ) -> Result<(), lookup::existing::Error>
    where
        Iter: IntoIterator<Item = Item>,
        Item: Into<&'a BStr>,
    {
        self.raw_values_mut_by(section_name, subsection_name, key.as_ref())
            .map(|mut v| v.set_values(new_values))
    }
}
