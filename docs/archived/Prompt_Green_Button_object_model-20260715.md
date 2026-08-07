The file @Toronto_Hydro_Object_Model.md was created using the greenbutton-objects library (see https://pypi.org/project/greenbutton-objects/) to understand the object model in the Toronto Hydro Green Button download file @TH_Electric_Usage_23-11-2024_to_24-06-2026.XML. Read enough of the XML file to be able to glean the specifics of the Toronto Hydro object model, not the entire file if possible.

Use the `uv` Python version and package manager (https://docs.astral.sh/uv/concepts/projects/init/) for the installation of any required Python versions and packages. Consider using the existing @explore_model.py or replace it as needed.

Update @Toronto_Hydro_Object_Model.md, adding the following:

- Any corrections that may be required.
- The UML diagram was intended to represent the domain model, not necessarily the Python classes used in the library. Adjust it as needed.
- Description of how each of the 3 MeterReadings types is encoded in the XML file and examples of lines from the file showing both the encoded data and its decoding.
- Description of how the time of reading is encoded in the XML file and example line from the file showing both the encoded data and its decoding. In particular, if the time of reading is not in UTC then how does the data account for duplicated times on the day DST begins?
- Description of how the different domain objects in the UML diagram map to sections of the XML file.

=====

What is the "Sample IntervalReading (first reading, KWH series)" section in the report? I can't find `timePeriod.start` anywhere in the XML file. That section of the document is not useful as-is. Fix it.

=====

The document says MeterReading appears as entries 6-8 but that is not correct. The first MeterReading appears as entry 6, followed by the KWH intervals. Fix this.
