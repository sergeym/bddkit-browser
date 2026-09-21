Feature: a page that logs an error

  Scenario: the broken widget is caught
    Given I am on "/broken"
    And I expect the next assertion to pass within "2" seconds
    Then the "h1" element should contain "This page logs an error"
    And the browser console should have no errors
